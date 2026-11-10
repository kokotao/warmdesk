/**
 * @description 跨平台采集 WarmDesk 所需的系统状态，并在硬件能力不可用时安全降级。
 * @author Albert_Luo
 * @email 480199976@qq.com
 * @date 2026-11-10 14:20
 */
use serde::Serialize;
use std::{
    io::Read,
    process::{Child, Command, Stdio},
    thread,
    time::{Duration, Instant},
};

const COMMAND_TIMEOUT: Duration = Duration::from_secs(3);

#[derive(Clone, Debug, Default, Serialize)]
pub struct MonitorSnapshot {
    pub cpu_usage_percent: Option<f32>,
    pub cpu_temperature_c: Option<f32>,
    pub thermal_warning: Option<bool>,
    pub battery_percent: Option<u8>,
    pub on_ac_power: Option<bool>,
    pub locked: bool,
    pub lock_state_available: bool,
    pub note: Option<String>,
}

pub fn sample() -> MonitorSnapshot {
    let mut snapshot = MonitorSnapshot::default();
    let mut notes = Vec::new();

    let (cpu_usage, cpu_note) = sample_cpu_usage();
    snapshot.cpu_usage_percent = cpu_usage;
    if let Some(note) = cpu_note {
        notes.push(note);
    }

    let (temperature, temperature_note) = sample_cpu_temperature();
    snapshot.cpu_temperature_c = temperature;
    if let Some(note) = temperature_note {
        notes.push(note);
    }

    let (thermal_warning, thermal_note) = sample_thermal_state();
    snapshot.thermal_warning = thermal_warning;
    if let Some(note) = thermal_note {
        notes.push(note);
    }

    let (battery, ac_power, battery_note) = sample_power();
    snapshot.battery_percent = battery;
    snapshot.on_ac_power = ac_power;
    if let Some(note) = battery_note {
        notes.push(note);
    }

    let (locked, lock_state_available, lock_note) = sample_lock_state();
    snapshot.locked = locked;
    snapshot.lock_state_available = lock_state_available;
    if let Some(note) = lock_note {
        notes.push(note);
    }

    snapshot.note = (!notes.is_empty()).then(|| notes.join("；"));
    snapshot
}

fn run_command(program: &str, args: &[&str]) -> Result<String, String> {
    let mut child = Command::new(program)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|error| format!("{program}: {error}"))?;

    let mut stdout = child
        .stdout
        .take()
        .ok_or_else(|| format!("{program}: stdout unavailable"))?;
    let reader = thread::spawn(move || {
        let mut output = Vec::new();
        stdout.read_to_end(&mut output).map(|_| output)
    });
    let deadline = Instant::now() + COMMAND_TIMEOUT;
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                let output = reader
                    .join()
                    .map_err(|_| format!("{program}: output reader stopped"))?
                    .map_err(|error| format!("{program}: {error}"))?;
                if status.success() {
                    return Ok(String::from_utf8_lossy(&output).into_owned());
                }
                return Err(format!("{program}: exited with {status}"));
            }
            Ok(None) if Instant::now() < deadline => {
                thread::sleep(Duration::from_millis(25));
            }
            Ok(None) => {
                terminate_child(&mut child);
                let _ = reader.join();
                return Err(format!("{program}: command timed out"));
            }
            Err(error) => {
                terminate_child(&mut child);
                let _ = reader.join();
                return Err(format!("{program}: {error}"));
            }
        }
    }
}

fn terminate_child(child: &mut Child) {
    let _ = child.kill();
    let _ = child.wait();
}

fn first_number(text: &str) -> Option<f32> {
    text.split(|character: char| {
        !(character.is_ascii_digit() || matches!(character, '.' | '-' | '+'))
    })
    .filter(|part| !part.is_empty() && *part != "+" && *part != "-")
    .find_map(|part| part.parse::<f32>().ok())
}

fn parse_cpu_percent(text: &str) -> Option<f32> {
    let value = first_number(text)?;
    (value.is_finite() && (0.0..=100.0).contains(&value)).then_some(value)
}

fn parse_macos_top_cpu(text: &str) -> Option<f32> {
    let usage = text
        .lines()
        .find(|line| line.to_ascii_lowercase().contains("cpu usage"))?;
    let idle = usage
        .split(',')
        .find(|part| part.to_ascii_lowercase().contains("idle"))?;
    let idle_percent = first_number(idle)?;
    let busy_percent = 100.0 - idle_percent;
    (busy_percent.is_finite() && (0.0..=100.0).contains(&busy_percent)).then_some(busy_percent)
}

fn parse_macos_battery(text: &str) -> (Option<u8>, Option<bool>) {
    let battery = text.lines().find_map(|line| {
        let percent = line.find('%')?;
        let digits = line[..percent]
            .chars()
            .rev()
            .take_while(|character| character.is_ascii_digit())
            .collect::<String>()
            .chars()
            .rev()
            .collect::<String>();
        let value = digits.parse::<u8>().ok()?;
        (value <= 100).then_some(value)
    });
    let lower = text.to_ascii_lowercase();
    let on_ac = if lower.contains("ac power") {
        Some(true)
    } else if lower.contains("battery power") {
        Some(false)
    } else if lower.contains("charging") || lower.contains("charged") {
        Some(true)
    } else {
        None
    };
    (battery, on_ac)
}

fn parse_windows_battery(text: &str) -> (Option<u8>, Option<bool>) {
    let mut fields = text.split('|');
    let battery = fields
        .next()
        .and_then(|value| value.trim().parse::<u8>().ok())
        .filter(|value| *value <= 100);
    let on_ac = fields.next().and_then(|value| match value.trim() {
        "0" => Some(false),
        "1" => Some(true),
        _ => None,
    });
    (battery, on_ac)
}

fn parse_temperature_c(text: &str) -> Option<f32> {
    let value = first_number(text)?;
    (value.is_finite() && (0.0..=120.0).contains(&value)).then_some(value)
}

fn parse_macos_thermal_warning(text: &str) -> Option<bool> {
    let lower = text.to_ascii_lowercase();
    if lower.contains("no thermal warning")
        && (lower.contains("no performance warning") || !lower.contains("performance warning"))
    {
        return Some(false);
    }
    if lower.contains("thermal warning")
        || lower.contains("cpu speed limit")
        || lower.contains("performance warning")
    {
        return Some(true);
    }
    None
}

fn parse_macos_lock_state(text: &str) -> Option<bool> {
    if text.contains("CGSSessionScreenIsLocked = 1") {
        Some(true)
    } else if text.contains("CGSSessionScreenIsLocked = 0") {
        Some(false)
    } else {
        None
    }
}

fn parse_windows_lock_state(text: &str) -> Option<bool> {
    text.trim()
        .parse::<u32>()
        .ok()
        .map(|process_count| process_count > 0)
}

#[cfg(target_os = "macos")]
fn sample_cpu_usage() -> (Option<f32>, Option<String>) {
    match run_command("top", &["-l", "2", "-n", "0", "-s", "1"]) {
        Ok(output) => (parse_macos_top_cpu(&output), None),
        Err(error) => (None, Some(format!("CPU 占用采样不可用：{error}"))),
    }
}

#[cfg(target_os = "windows")]
fn sample_cpu_usage() -> (Option<f32>, Option<String>) {
    let script =
        "(Get-Counter '\\Processor(_Total)\\% Processor Time').CounterSamples[0].CookedValue";
    match run_command(
        "powershell.exe",
        &["-NoProfile", "-NonInteractive", "-Command", script],
    ) {
        Ok(output) => (parse_cpu_percent(&output), None),
        Err(error) => (None, Some(format!("CPU 占用采样不可用：{error}"))),
    }
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
fn sample_cpu_usage() -> (Option<f32>, Option<String>) {
    (None, Some("当前平台暂未提供 CPU 占用采样".to_string()))
}

#[cfg(target_os = "macos")]
fn sample_cpu_temperature() -> (Option<f32>, Option<String>) {
    (
        None,
        Some("macOS 未提供稳定的公开 CPU 温度接口，已采用保守强度上限".to_string()),
    )
}

#[cfg(target_os = "macos")]
fn sample_thermal_state() -> (Option<bool>, Option<String>) {
    match run_command("pmset", &["-g", "therm"]) {
        Ok(output) => match parse_macos_thermal_warning(&output) {
            Some(true) => (Some(true), Some("系统热状态告警，暖手已降载".to_string())),
            Some(false) => (Some(false), None),
            None => (None, Some("系统热状态采样格式不可识别".to_string())),
        },
        Err(error) => (None, Some(format!("系统热状态采样不可用：{error}"))),
    }
}

#[cfg(target_os = "windows")]
fn sample_thermal_state() -> (Option<bool>, Option<String>) {
    (None, None)
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
fn sample_thermal_state() -> (Option<bool>, Option<String>) {
    (None, Some("当前平台暂未提供系统热状态采样".to_string()))
}

#[cfg(target_os = "windows")]
fn sample_cpu_temperature() -> (Option<f32>, Option<String>) {
    let script = "(Get-CimInstance MSAcpi_ThermalZoneTemperature -ErrorAction SilentlyContinue | Select-Object -First 1 -ExpandProperty CurrentTemperature)";
    match run_command(
        "powershell.exe",
        &["-NoProfile", "-NonInteractive", "-Command", script],
    ) {
        Ok(output) => {
            let temperature = first_number(&output).and_then(|kelvin_tenths| {
                let celsius = kelvin_tenths / 10.0 - 273.15;
                (celsius.is_finite() && (0.0..=120.0).contains(&celsius)).then_some(celsius)
            });
            match temperature {
                Some(value) => (Some(value), None),
                None => (
                    None,
                    Some("ACPI 热区读数不可用，已采用保守强度上限".to_string()),
                ),
            }
        }
        Err(_) => (
            None,
            Some("ACPI 热区读数不可用，已采用保守强度上限".to_string()),
        ),
    }
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
fn sample_cpu_temperature() -> (Option<f32>, Option<String>) {
    (None, Some("当前平台暂未提供 CPU 温度采样".to_string()))
}

#[cfg(target_os = "macos")]
fn sample_power() -> (Option<u8>, Option<bool>, Option<String>) {
    match run_command("pmset", &["-g", "batt"]) {
        Ok(output) => {
            let (battery, on_ac) = parse_macos_battery(&output);
            (battery, on_ac, None)
        }
        Err(error) => (None, None, Some(format!("电源状态采样不可用：{error}"))),
    }
}

#[cfg(target_os = "windows")]
fn sample_power() -> (Option<u8>, Option<bool>, Option<String>) {
    let script = r#"
Add-Type -TypeDefinition @'
using System;
using System.Runtime.InteropServices;
[StructLayout(LayoutKind.Sequential)]
public struct WarmDeskPowerStatus {
    public byte ACLineStatus;
    public byte BatteryFlag;
    public byte BatteryLifePercent;
    public byte SystemStatusFlag;
    public int BatteryLifeTime;
    public int BatteryFullLifeTime;
}
public static class WarmDeskPower {
    [DllImport("kernel32.dll")]
    public static extern bool GetSystemPowerStatus(out WarmDeskPowerStatus status);
}
'@ -ErrorAction SilentlyContinue
$status = New-Object WarmDeskPowerStatus
if ([WarmDeskPower]::GetSystemPowerStatus([ref]$status)) {
    Write-Output "$($status.BatteryLifePercent)|$($status.ACLineStatus)"
} else {
    exit 1
}
"#;
    match run_command(
        "powershell.exe",
        &["-NoProfile", "-NonInteractive", "-Command", script],
    ) {
        Ok(output) => {
            let (battery, on_ac) = parse_windows_battery(output.trim());
            (battery, on_ac, None)
        }
        Err(error) => (None, None, Some(format!("电源状态采样不可用：{error}"))),
    }
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
fn sample_power() -> (Option<u8>, Option<bool>, Option<String>) {
    (None, None, Some("当前平台暂未提供电源状态采样".to_string()))
}

#[cfg(target_os = "macos")]
fn sample_lock_state() -> (bool, bool, Option<String>) {
    let cg_session =
        "/System/Library/CoreServices/Menu Extras/User.menu/Contents/Resources/CGSession";
    match run_command(cg_session, &["-s"]) {
        Ok(output) => parse_macos_lock_state(&output).map_or_else(
            || {
                (
                    false,
                    false,
                    Some("锁屏状态无法确认，暖手应暂停".to_string()),
                )
            },
            |locked| (locked, true, None),
        ),
        Err(_) => (false, false, Some("锁屏状态不可用，暖手应暂停".to_string())),
    }
}

#[cfg(target_os = "windows")]
fn sample_lock_state() -> (bool, bool, Option<String>) {
    match run_command(
        "powershell.exe",
        &[
            "-NoProfile",
            "-NonInteractive",
            "-Command",
            "(Get-Process -Name LogonUI -ErrorAction SilentlyContinue | Measure-Object).Count",
        ],
    ) {
        Ok(output) => parse_windows_lock_state(&output).map_or_else(
            || {
                (
                    false,
                    false,
                    Some("锁屏状态无法确认，暖手应暂停".to_string()),
                )
            },
            |locked| (locked, true, None),
        ),
        Err(_) => (false, false, Some("锁屏状态不可用，暖手应暂停".to_string())),
    }
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
fn sample_lock_state() -> (bool, bool, Option<String>) {
    (
        false,
        false,
        Some("当前平台暂未提供锁屏状态采样，暖手应暂停".to_string()),
    )
}

#[cfg(test)]
mod tests {
    use super::{
        parse_cpu_percent, parse_macos_battery, parse_macos_lock_state,
        parse_macos_thermal_warning, parse_macos_top_cpu, parse_temperature_c,
        parse_windows_battery, parse_windows_lock_state,
    };

    #[test]
    fn parses_cpu_percent_with_bounds() {
        assert_eq!(parse_cpu_percent(" 37.5\r\n"), Some(37.5));
        assert_eq!(parse_cpu_percent("101"), None);
    }

    #[test]
    fn parses_macos_system_cpu_idle_percentage() {
        assert_eq!(
            parse_macos_top_cpu("CPU usage: 12.5% user, 8.0% sys, 79.5% idle\n"),
            Some(20.5)
        );
        assert_eq!(parse_macos_top_cpu("no CPU data"), None);
    }

    #[test]
    fn parses_macos_battery_and_ac_state() {
        assert_eq!(
            parse_macos_battery(
                "Now drawing from 'AC Power'\n-InternalBattery-0 (id=123) 84%; charging; 1:22 remaining"
            ),
            (Some(84), Some(true))
        );
        assert_eq!(
            parse_macos_battery(
                "Now drawing from 'Battery Power'\n-InternalBattery-0 19%; discharging"
            ),
            (Some(19), Some(false))
        );
    }

    #[test]
    fn parses_windows_battery_status() {
        assert_eq!(parse_windows_battery("73|1"), (Some(73), Some(true)));
        assert_eq!(parse_windows_battery("12|0"), (Some(12), Some(false)));
        assert_eq!(parse_windows_battery("255|1"), (None, Some(true)));
        assert_eq!(parse_windows_battery("unknown|"), (None, None));
    }

    #[test]
    fn rejects_unrealistic_temperature() {
        assert_eq!(parse_temperature_c("Temperature: 54.2"), Some(54.2));
        assert_eq!(parse_temperature_c("Temperature: 140"), None);
    }

    #[test]
    fn parses_macos_thermal_warning() {
        assert_eq!(
            parse_macos_thermal_warning(
                "Note: No thermal warning level has been recorded\n\
                 Note: No performance warning level has been recorded"
            ),
            Some(false)
        );
        assert_eq!(
            parse_macos_thermal_warning("CPU Speed Limit = 80"),
            Some(true)
        );
        assert_eq!(parse_macos_thermal_warning(""), None);
    }

    #[test]
    fn distinguishes_lock_state_from_unavailable_output() {
        assert_eq!(
            parse_macos_lock_state("CGSSessionScreenIsLocked = 1"),
            Some(true)
        );
        assert_eq!(
            parse_macos_lock_state("CGSSessionScreenIsLocked = 0"),
            Some(false)
        );
        assert_eq!(parse_macos_lock_state(""), None);
        assert_eq!(parse_windows_lock_state("2"), Some(true));
        assert_eq!(parse_windows_lock_state("0"), Some(false));
        assert_eq!(parse_windows_lock_state("not a number"), None);
    }
}
