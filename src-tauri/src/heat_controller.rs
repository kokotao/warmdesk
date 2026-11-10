/**
 * @description 通过可取消、分时让出 CPU 的工作负载产生设备余热，并执行运行时安全保护。
 * @author Albert_Luo
 * @email 480199976@qq.com
 * @date 2026-11-10 14:35
 */
use crate::platform_monitor::{self, MonitorSnapshot};
use serde::{Deserialize, Serialize};
use std::{
    hint::black_box,
    sync::{
        atomic::{AtomicBool, AtomicU8, Ordering},
        Arc, Mutex,
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

const MAX_INTENSITY: u8 = 60;
const GENTLE_INTENSITY: u8 = 10;
const CYCLE_MS: u64 = 100;
const SAMPLE_INTERVAL: Duration = Duration::from_secs(5);

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum HeatMode {
    Steady,
    Pulse,
    Adaptive,
}

impl Default for HeatMode {
    fn default() -> Self {
        Self::Steady
    }
}

#[derive(Clone, Copy, Debug)]
pub struct SafetyLimits {
    pub max_intensity: u8,
    pub cpu_pressure: u8,
    pub locked: bool,
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct WarmStatus {
    pub running: bool,
    pub intensity: u8,
    pub mode: HeatMode,
    pub elapsed_seconds: u64,
    pub remaining_seconds: Option<u64>,
    pub cpu_usage_percent: Option<f32>,
    pub cpu_temperature_c: Option<f32>,
    pub battery_percent: Option<u8>,
    pub on_ac_power: Option<bool>,
    pub locked: bool,
    pub lock_state_available: bool,
    pub protection_reason: Option<String>,
    pub monitor_note: Option<String>,
}

struct ControllerState {
    status: WarmStatus,
    duration: Option<Duration>,
    started_at: Option<Instant>,
}

struct WorkerHandles {
    heat: JoinHandle<()>,
}

pub struct HeatController {
    state: Arc<Mutex<ControllerState>>,
    stop_signal: Arc<AtomicBool>,
    duty: Arc<AtomicU8>,
    monitor_shutdown: Arc<AtomicBool>,
    monitor: Mutex<Option<JoinHandle<()>>>,
    worker: Mutex<Option<WorkerHandles>>,
}

impl Default for HeatController {
    fn default() -> Self {
        Self::new()
    }
}

impl HeatController {
    pub fn new() -> Self {
        let state = Arc::new(Mutex::new(ControllerState {
            status: WarmStatus {
                intensity: 28,
                mode: HeatMode::Steady,
                ..WarmStatus::default()
            },
            duration: None,
            started_at: None,
        }));
        let stop_signal = Arc::new(AtomicBool::new(false));
        let duty = Arc::new(AtomicU8::new(0));
        let monitor_shutdown = Arc::new(AtomicBool::new(false));
        let monitor_state = Arc::clone(&state);
        let monitor_stop = Arc::clone(&stop_signal);
        let monitor_duty = Arc::clone(&duty);
        let shutdown_signal = Arc::clone(&monitor_shutdown);
        let monitor = thread::Builder::new()
            .name("warmdesk-monitor".to_string())
            .spawn(move || {
                run_supervisor(monitor_state, monitor_stop, monitor_duty, shutdown_signal)
            })
            .ok();
        Self {
            state,
            stop_signal,
            duty,
            monitor_shutdown,
            monitor: Mutex::new(monitor),
            worker: Mutex::new(None),
        }
    }

    pub fn status(&self) -> WarmStatus {
        self.state
            .lock()
            .map(|state| state.status.clone())
            .unwrap_or_default()
    }

    pub fn start(
        &self,
        intensity: u8,
        mode: HeatMode,
        duration_minutes: u32,
    ) -> Result<WarmStatus, String> {
        if self.status().running {
            return Err("暖手任务已在运行中".to_string());
        }

        self.stop_signal.store(false, Ordering::Release);
        self.duty.store(GENTLE_INTENSITY, Ordering::Release);
        let duration =
            (duration_minutes > 0).then(|| Duration::from_secs(u64::from(duration_minutes) * 60));
        let started_at = Instant::now();
        let initial_snapshot = MonitorSnapshot::default();
        let normalized_intensity = intensity.clamp(5, MAX_INTENSITY);

        {
            let mut state = self
                .state
                .lock()
                .map_err(|_| "暖手状态锁不可用".to_string())?;
            state.duration = duration;
            state.started_at = Some(started_at);
            state.status = status_from_snapshot(
                true,
                normalized_intensity,
                mode,
                0,
                duration.map(|value| value.as_secs()),
                &initial_snapshot,
                None,
            );
        }

        let stop_signal = Arc::clone(&self.stop_signal);
        let duty = Arc::clone(&self.duty);
        let worker = thread::Builder::new()
            .name("warmdesk-heat-worker".to_string())
            .spawn(move || run_worker(duty, stop_signal))
            .map_err(|error| format!("暖手线程启动失败：{error}"))?;

        self.worker
            .lock()
            .map_err(|_| "暖手线程句柄不可用".to_string())?
            .replace(WorkerHandles { heat: worker });
        Ok(self.status())
    }

    pub fn update(&self, intensity: u8, mode: HeatMode) -> Result<WarmStatus, String> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| "暖手状态锁不可用".to_string())?;
        if !state.status.running {
            return Err("暖手任务当前未运行".to_string());
        }
        state.status.intensity = intensity.clamp(5, MAX_INTENSITY);
        state.status.mode = mode;
        Ok(state.status.clone())
    }

    pub fn stop(&self) -> WarmStatus {
        self.stop_signal.store(true, Ordering::Release);
        self.duty.store(0, Ordering::Release);
        if let Ok(mut worker) = self.worker.lock() {
            if let Some(handles) = worker.take() {
                let _ = handles.heat.join();
            }
        }
        if let Ok(mut state) = self.state.lock() {
            state.status.running = false;
            state.started_at = None;
            return state.status.clone();
        }
        WarmStatus::default()
    }
}

impl Drop for HeatController {
    fn drop(&mut self) {
        self.monitor_shutdown.store(true, Ordering::Release);
        self.stop_signal.store(true, Ordering::Release);
        self.duty.store(0, Ordering::Release);
        if let Ok(monitor) = self.monitor.get_mut() {
            drop(monitor.take());
        }
        if let Ok(worker) = self.worker.get_mut() {
            if let Some(handles) = worker.take() {
                let _ = handles.heat.join();
            }
        }
    }
}

fn run_supervisor(
    state: Arc<Mutex<ControllerState>>,
    stop_signal: Arc<AtomicBool>,
    duty: Arc<AtomicU8>,
    shutdown: Arc<AtomicBool>,
) {
    let mut last_sample = Instant::now() - SAMPLE_INTERVAL;
    let mut snapshot = MonitorSnapshot::default();
    let mut phase_seconds = 0_u64;

    while !shutdown.load(Ordering::Acquire) {
        if last_sample.elapsed() >= SAMPLE_INTERVAL {
            snapshot = platform_monitor::sample();
            last_sample = Instant::now();
        }

        let mut hard_stop = None;
        if let Ok(mut state) = state.lock() {
            let elapsed = state
                .started_at
                .map(|started_at| started_at.elapsed().as_secs())
                .unwrap_or_default();
            state.status.elapsed_seconds = elapsed;
            state.status.remaining_seconds = state
                .duration
                .map(|duration| duration.as_secs().saturating_sub(elapsed));
            state.status.cpu_usage_percent = snapshot.cpu_usage_percent;
            state.status.cpu_temperature_c = snapshot.cpu_temperature_c;
            state.status.battery_percent = snapshot.battery_percent;
            state.status.on_ac_power = snapshot.on_ac_power;
            state.status.locked = snapshot.locked;
            state.status.lock_state_available = snapshot.lock_state_available;
            state.status.monitor_note = snapshot.note.clone();

            if state.status.running {
                if state
                    .duration
                    .is_some_and(|duration| elapsed >= duration.as_secs())
                {
                    hard_stop = Some("定时结束".to_string());
                } else {
                    hard_stop = evaluate_protection(&snapshot).hard_stop;
                }
            }

            if let Some(reason) = hard_stop.as_ref() {
                state.status.running = false;
                state.status.protection_reason = Some(reason.clone());
                state.started_at = None;
            } else if state.status.running {
                let limits = evaluate_protection(&snapshot);
                duty.store(
                    calculate_duty(
                        state.status.intensity,
                        state.status.mode,
                        SafetyLimits {
                            max_intensity: limits.max_intensity,
                            cpu_pressure: snapshot.cpu_usage_percent.unwrap_or_default().round()
                                as u8,
                            locked: snapshot.locked,
                        },
                        phase_seconds,
                    ),
                    Ordering::Release,
                );
            } else {
                duty.store(0, Ordering::Release);
            }
        } else {
            stop_signal.store(true, Ordering::Release);
            break;
        }

        if hard_stop.is_some() {
            duty.store(0, Ordering::Release);
            stop_signal.store(true, Ordering::Release);
        }
        phase_seconds = phase_seconds.saturating_add(1);
        thread::sleep(Duration::from_millis(CYCLE_MS));
    }
}

fn run_worker(duty: Arc<AtomicU8>, stop_signal: Arc<AtomicBool>) {
    while !stop_signal.load(Ordering::Acquire) {
        let cycle_start = Instant::now();
        do_work_slice(duty.load(Ordering::Acquire));
        let cycle_elapsed = cycle_start.elapsed();
        if cycle_elapsed < Duration::from_millis(CYCLE_MS) {
            sleep_while_active(
                Duration::from_millis(CYCLE_MS) - cycle_elapsed,
                &stop_signal,
            );
        }
    }
}

fn sleep_while_active(duration: Duration, stop_signal: &AtomicBool) {
    let deadline = Instant::now() + duration;
    while !stop_signal.load(Ordering::Acquire) {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            break;
        }
        thread::sleep(remaining.min(Duration::from_millis(50)));
    }
}

struct Protection {
    max_intensity: u8,
    hard_stop: Option<String>,
}

fn evaluate_protection(snapshot: &MonitorSnapshot) -> Protection {
    if snapshot
        .cpu_usage_percent
        .is_some_and(|value| value >= 95.0)
    {
        return Protection {
            max_intensity: 0,
            hard_stop: Some("系统 CPU 已接近满载，暖手已暂停".to_string()),
        };
    }
    if snapshot
        .cpu_temperature_c
        .is_some_and(|value| value >= 85.0)
    {
        return Protection {
            max_intensity: 0,
            hard_stop: Some("检测到 CPU 温度过高，暖手已暂停".to_string()),
        };
    }
    if snapshot.battery_percent.is_some_and(|value| value < 10) {
        return Protection {
            max_intensity: 0,
            hard_stop: Some("电池电量低于 10%，暖手已暂停".to_string()),
        };
    }

    let mut max_intensity = MAX_INTENSITY;
    if snapshot
        .cpu_usage_percent
        .is_some_and(|value| value >= 85.0)
        || snapshot
            .cpu_temperature_c
            .is_some_and(|value| value >= 75.0)
        || snapshot.battery_percent.is_some_and(|value| value < 20)
        || snapshot.locked
        || snapshot.cpu_temperature_c.is_none()
    {
        max_intensity = 25;
    }
    if snapshot.cpu_usage_percent.is_none()
        || !snapshot.lock_state_available
        || snapshot.on_ac_power.is_none()
    {
        max_intensity = max_intensity.min(GENTLE_INTENSITY);
    }
    Protection {
        max_intensity,
        hard_stop: None,
    }
}

pub fn calculate_duty(
    requested_intensity: u8,
    mode: HeatMode,
    limits: SafetyLimits,
    phase_seconds: u64,
) -> u8 {
    let mut duty = requested_intensity
        .min(limits.max_intensity)
        .min(MAX_INTENSITY);
    if limits.cpu_pressure >= 95 {
        return 0;
    }
    if limits.cpu_pressure >= 85 {
        duty = duty.min(GENTLE_INTENSITY);
    } else if limits.cpu_pressure >= 70 && mode == HeatMode::Adaptive {
        duty /= 2;
    }
    if limits.locked {
        duty = duty.min(GENTLE_INTENSITY);
    }
    if mode == HeatMode::Pulse && phase_seconds % 10 >= 5 {
        return 0;
    }
    duty
}

fn do_work_slice(duty: u8) {
    let active_ms = CYCLE_MS.saturating_mul(u64::from(duty)) / 100;
    if active_ms == 0 {
        return;
    }
    let deadline = Instant::now() + Duration::from_millis(active_ms);
    let mut value = 0_u64;
    while Instant::now() < deadline {
        for _ in 0..4096 {
            value = value
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
        }
        black_box(value);
    }
}

fn status_from_snapshot(
    running: bool,
    intensity: u8,
    mode: HeatMode,
    elapsed_seconds: u64,
    remaining_seconds: Option<u64>,
    snapshot: &MonitorSnapshot,
    protection_reason: Option<String>,
) -> WarmStatus {
    WarmStatus {
        running,
        intensity,
        mode,
        elapsed_seconds,
        remaining_seconds,
        cpu_usage_percent: snapshot.cpu_usage_percent,
        cpu_temperature_c: snapshot.cpu_temperature_c,
        battery_percent: snapshot.battery_percent,
        on_ac_power: snapshot.on_ac_power,
        locked: snapshot.locked,
        lock_state_available: snapshot.lock_state_available,
        protection_reason,
        monitor_note: snapshot.note.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::{calculate_duty, HeatMode, SafetyLimits};

    #[test]
    fn clamps_requested_intensity_to_platform_limit() {
        assert_eq!(
            calculate_duty(
                60,
                HeatMode::Steady,
                SafetyLimits {
                    max_intensity: 25,
                    cpu_pressure: 0,
                    locked: false,
                },
                0
            ),
            25
        );
    }

    #[test]
    fn reduces_budget_when_system_cpu_is_busy() {
        assert_eq!(
            calculate_duty(
                40,
                HeatMode::Steady,
                SafetyLimits {
                    max_intensity: 60,
                    cpu_pressure: 88,
                    locked: false,
                },
                0
            ),
            10
        );
    }

    #[test]
    fn locked_session_is_limited_to_gentle_budget() {
        assert_eq!(
            calculate_duty(
                60,
                HeatMode::Steady,
                SafetyLimits {
                    max_intensity: 60,
                    cpu_pressure: 0,
                    locked: true,
                },
                0
            ),
            10
        );
    }

    #[test]
    fn pulse_mode_is_periodic_and_adaptive_mode_responds_to_load() {
        let limits = SafetyLimits {
            max_intensity: 60,
            cpu_pressure: 0,
            locked: false,
        };
        assert_eq!(calculate_duty(40, HeatMode::Pulse, limits, 0), 40);
        assert_eq!(calculate_duty(40, HeatMode::Pulse, limits, 5), 0);
        assert_eq!(calculate_duty(40, HeatMode::Adaptive, limits, 0), 40);
        assert_eq!(
            calculate_duty(
                40,
                HeatMode::Adaptive,
                SafetyLimits {
                    cpu_pressure: 75,
                    ..limits
                },
                0
            ),
            20
        );
    }
}
