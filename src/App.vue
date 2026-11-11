<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import { listen } from '@tauri-apps/api/event'
import {
  Activity,
  BatteryCharging,
  ChevronDown,
  Clock3,
  ExternalLink,
  LockKeyhole,
  Moon,
  Pause,
  Play,
  Power,
  RefreshCw,
  ShieldCheck,
  Sun,
  Thermometer,
  TimerReset,
  Zap
} from '@lucide/vue'

type Mode = 'steady' | 'pulse' | 'adaptive'
type Theme = 'light' | 'dark' | 'system'

interface WarmStatus {
  running: boolean
  intensity: number
  mode: Mode
  elapsed_seconds: number
  remaining_seconds: number | null
  cpu_usage_percent: number | null
  cpu_temperature_c: number | null
  thermal_warning: boolean | null
  battery_percent: number | null
  on_ac_power: boolean | null
  locked: boolean
  lock_state_available: boolean
  protection_reason: string | null
  monitor_note: string | null
}

const running = ref(false)
const intensity = ref(28)
const mode = ref<Mode>('steady')
const duration = ref(60)
const elapsed = ref(0)
const remaining = ref<number | null>(null)
const cpuUsage = ref<number | null>(null)
const cpuTemperature = ref<number | null>(null)
const thermalWarning = ref<boolean | null>(null)
const batteryPercent = ref<number | null>(null)
const onAcPower = ref<boolean | null>(null)
const locked = ref(false)
const lockStateAvailable = ref(false)
const protectionReason = ref<string | null>(null)
const monitorNote = ref<string | null>(null)
const desktopRuntimeAvailable = ref(true)
const busy = ref(false)
const commandError = ref('')
const theme = ref<Theme>('system')
const showSettings = ref(false)
const currentVersion = ref('0.2.2')
const updateStatus = ref<'idle' | 'checking' | 'up-to-date' | 'available' | 'error'>('idle')
const updateMessage = ref('尚未检查')
const latestVersion = ref('')
const releaseUrl = ref('https://github.com/kokotao/warmdesk/releases/latest')
const releaseApiUrl = ref('https://api.github.com/repos/kokotao/warmdesk/releases/latest')
const mirrorManifestUrl = ref('https://note.lizipro.cn/warmdesk/releases/latest.json')
const downloadSource = ref<'mirror' | 'github'>('github')
const platformOs = ref('')
const platformArch = ref('')
const downloadState = ref<'idle' | 'downloading' | 'completed' | 'error'>('idle')
const downloadProgress = ref(0)
const downloadMessage = ref('')
const selectedAsset = ref<{
  name: string
  url: string
  size?: number
  sha256?: string
} | null>(null)
let tick: number | undefined
let statusPoll: number | undefined
let colorScheme: MediaQueryList | undefined
let unlistenDownload: (() => void) | undefined

const modeLabel = computed(() => ({
  steady: '恒温',
  pulse: '脉冲',
  adaptive: '自适应'
}[mode.value]))

function isHttpsUrl(value: string | undefined): value is string {
  if (!value) return false
  try {
    return new URL(value).protocol === 'https:'
  } catch {
    return false
  }
}

function selectAsset<T extends {
  name?: string
  size?: number
  sha256?: string
  digest?: string
  urls?: { note?: string; github?: string }
  browser_download_url?: string
}>(
  assets: T[] | undefined,
  isMirrorAsset: boolean,
  os = platformOs.value,
  arch = platformArch.value
) {
  const windows = os === 'windows' || (!os && /windows/i.test(navigator.userAgent))
  const arm64 = arch === 'aarch64' || arch === 'arm64'
  const candidates = assets?.filter((asset) => {
    const name = asset.name?.toLowerCase() || ''
    const packageMatch = windows ? name.endsWith('.msi') || name.endsWith('.exe') : name.endsWith('.dmg')
    const architectureMatch = windows
      ? !arm64 || name.includes('arm64') || name.includes('aarch64')
      : arm64 ? name.includes('aarch64') || name.includes('arm64') : name.includes('x64') || name.includes('x86_64')
    return packageMatch && architectureMatch
  }) ?? []
  const asset = candidates.find((item) => {
    const name = item.name?.toLowerCase() || ''
    return windows ? name.endsWith('.msi') : name.endsWith('.dmg')
  }) ?? candidates[0]
  const url = isMirrorAsset ? asset?.urls?.note : asset?.browser_download_url
  if (!asset?.name || !isHttpsUrl(url)) return null
  return {
    name: asset.name,
    url,
    size: asset.size,
    sha256: asset.sha256 || asset.digest?.replace(/^sha256:/, '')
  }
}

const elapsedLabel = computed(() => {
  const hours = Math.floor(elapsed.value / 3600)
  const minutes = Math.floor((elapsed.value % 3600) / 60)
  const seconds = elapsed.value % 60
  return [hours, minutes, seconds].map((part) => String(part).padStart(2, '0')).join(':')
})

const remainingLabel = computed(() => {
  if (remaining.value === null) return '不限时'
  const minutes = Math.floor(remaining.value / 60)
  const seconds = remaining.value % 60
  return `${minutes}分${String(seconds).padStart(2, '0')}秒`
})

const statusLabel = computed(() => {
  if (!desktopRuntimeAvailable.value) return '桌面运行时不可用'
  if (protectionReason.value) return '保护已介入'
  if (locked.value) return '已锁屏 · 已暂停'
  return running.value ? '正在暖手' : '状态 · 待命'
})

const safetyMessage = computed(() => {
  if (protectionReason.value) return protectionReason.value
  if (monitorNote.value) return monitorNote.value
  return '暖手通过分时 CPU 工作负载产生余热，会消耗电量；软件会在过热、系统繁忙或锁屏时降载/暂停。'
})

const displayTemperature = computed(() => cpuTemperature.value === null ? '--' : `${cpuTemperature.value.toFixed(0)}°`)
const temperatureCaption = computed(() => cpuTemperature.value === null ? 'CPU 温度不可用' : 'CPU 温度采样')
const thermalLabel = computed(() => thermalWarning.value === null ? '未知' : thermalWarning.value ? '告警' : '正常')
const displayCpu = computed(() => cpuUsage.value === null ? '--' : `${cpuUsage.value.toFixed(0)}%`)
const displayBattery = computed(() => batteryPercent.value === null ? '--' : `${batteryPercent.value.toFixed(0)}%`)
const powerLabel = computed(() => {
  const source = onAcPower.value === null ? '未知' : onAcPower.value ? '接通电源' : '电池供电'
  return batteryPercent.value === null ? source : `${source} · ${displayBattery.value}`
})

function applyStatus(status: WarmStatus) {
  running.value = status.running
  intensity.value = status.intensity
  mode.value = status.mode
  elapsed.value = status.elapsed_seconds
  remaining.value = status.remaining_seconds
  cpuUsage.value = status.cpu_usage_percent
  cpuTemperature.value = status.cpu_temperature_c
  thermalWarning.value = status.thermal_warning
  batteryPercent.value = status.battery_percent
  onAcPower.value = status.on_ac_power
  locked.value = status.locked
  lockStateAvailable.value = status.lock_state_available
  protectionReason.value = status.protection_reason
  monitorNote.value = status.monitor_note
}

async function refreshStatus() {
  try {
    const status = await invoke<WarmStatus>('get_warm_status')
    desktopRuntimeAvailable.value = true
    commandError.value = ''
    applyStatus(status)
  } catch {
    desktopRuntimeAvailable.value = false
  }
}

async function toggleRunning() {
  if (!desktopRuntimeAvailable.value || busy.value) return
  busy.value = true
  commandError.value = ''
  try {
    const status = running.value
      ? await invoke<WarmStatus>('stop_warming')
      : await invoke<WarmStatus>('start_warming', {
          intensity: intensity.value,
          mode: mode.value,
          durationMinutes: duration.value
        })
    applyStatus(status)
  } catch (error) {
    commandError.value = error instanceof Error ? error.message : '暖手任务未能启动，请检查安全状态。'
    await refreshStatus()
  } finally {
    busy.value = false
  }
}

async function updateWarmSettings(next: Partial<Pick<WarmStatus, 'intensity' | 'mode'>>) {
  if (!running.value || !desktopRuntimeAvailable.value || busy.value) return
  busy.value = true
  commandError.value = ''
  try {
    const status = await invoke<WarmStatus>('update_warm_settings', {
      intensity: next.intensity ?? intensity.value,
      mode: next.mode ?? mode.value
    })
    applyStatus(status)
  } catch (error) {
    commandError.value = error instanceof Error ? error.message : '暖手参数更新失败。'
    await refreshStatus()
  } finally {
    busy.value = false
  }
}

async function resetSession() {
  if (running.value) await toggleRunning()
  elapsed.value = 0
  remaining.value = duration.value > 0 ? duration.value * 60 : null
}

async function setTheme(nextTheme: Theme) {
  theme.value = nextTheme
  applyTheme()
  localStorage.setItem('warmdesk-theme', nextTheme)
  try {
    await invoke('set_theme', { theme: nextTheme })
  } catch {
    // Browser preview can still apply the theme locally.
  }
}

function applyTheme() {
  document.documentElement.dataset.theme = theme.value === 'system'
    ? colorScheme?.matches ? 'dark' : 'light'
    : theme.value
}

function clearTimer() {
  if (tick !== undefined) {
    window.clearInterval(tick)
    tick = undefined
  }
  if (statusPoll !== undefined) {
    window.clearInterval(statusPoll)
    statusPoll = undefined
  }
}

function compareVersions(left: string, right: string) {
  const normalize = (value: string) => value.replace(/^v/i, '').split('.').map((part) => Number.parseInt(part, 10) || 0)
  const a = normalize(left)
  const b = normalize(right)
  for (let index = 0; index < Math.max(a.length, b.length); index += 1) {
    if ((a[index] ?? 0) !== (b[index] ?? 0)) return (a[index] ?? 0) > (b[index] ?? 0) ? 1 : -1
  }
  return 0
}

async function loadUpdateInfo() {
  try {
    const info = await invoke<{
      current_version: string
      os?: string
      arch?: string
      mirror_manifest_url: string
      release_api_url: string
      release_page_url: string
    }>('get_update_info')
    currentVersion.value = info.current_version
    platformOs.value = info.os || ''
    platformArch.value = info.arch || ''
    mirrorManifestUrl.value = info.mirror_manifest_url
    releaseApiUrl.value = info.release_api_url
    releaseUrl.value = info.release_page_url
  } catch {
    // Browser preview does not have the Tauri command bridge; keep safe defaults.
  }
}

async function checkForUpdates() {
  updateStatus.value = 'checking'
  updateMessage.value = '正在检查栗子镜像…'
  downloadState.value = 'idle'
  downloadProgress.value = 0
  downloadMessage.value = ''
  try {
    let releaseVersion = ''
    let resolvedReleaseUrl = releaseUrl.value
    let resolvedDownloadUrl = ''
    try {
      const mirrorUrl = `${mirrorManifestUrl.value}?warmdesk_check=${Date.now()}`
      const mirrorResponse = await fetch(mirrorUrl, {
        cache: 'no-store',
        headers: { Accept: 'application/json', 'Cache-Control': 'no-cache' }
      })
      if (!mirrorResponse.ok) throw new Error(`HTTP ${mirrorResponse.status}`)
      const manifest = await mirrorResponse.json() as {
        version?: string
        tag?: string
        release_url?: string
        assets?: Array<{
          name?: string
          size?: number
          sha256?: string
          urls?: { note?: string; github?: string }
        }>
      }
      if (!manifest.version || !Array.isArray(manifest.assets)) throw new Error('镜像 manifest 无效')
      releaseVersion = manifest.tag || manifest.version
      resolvedReleaseUrl = manifest.release_url || resolvedReleaseUrl
      const mirrorAsset = selectAsset(manifest.assets, true)
      const githubAsset = selectAsset(manifest.assets, false)
      const asset = mirrorAsset || githubAsset
      selectedAsset.value = asset
      resolvedDownloadUrl = asset?.url || ''
      downloadSource.value = mirrorAsset ? 'mirror' : 'github'
    } catch {
      const response = await fetch(releaseApiUrl.value, {
        headers: { Accept: 'application/vnd.github+json' }
      })
      if (!response.ok) throw new Error(`HTTP ${response.status}`)
      const release = await response.json() as {
        tag_name?: string
        html_url?: string
        assets?: Array<{ browser_download_url?: string; name?: string; size?: number; digest?: string }>
        prerelease?: boolean
        draft?: boolean
      }
      if (!release.tag_name || release.draft || release.prerelease) throw new Error('暂无稳定版本')
      releaseVersion = release.tag_name
      resolvedReleaseUrl = release.html_url || resolvedReleaseUrl
      const asset = selectAsset(release.assets, false)
      selectedAsset.value = asset
      resolvedDownloadUrl = asset?.url || ''
      downloadSource.value = 'github'
    }

    latestVersion.value = releaseVersion
    releaseUrl.value = resolvedDownloadUrl || resolvedReleaseUrl
    if (compareVersions(releaseVersion, currentVersion.value) > 0) {
      updateStatus.value = 'available'
      updateMessage.value = `发现新版本 ${releaseVersion} · ${downloadSource.value === 'mirror' ? '栗子镜像' : 'GitHub'}`
    } else {
      updateStatus.value = 'up-to-date'
      updateMessage.value = `当前已是最新版本（${currentVersion.value}）· ${downloadSource.value === 'mirror' ? '栗子镜像' : 'GitHub'}`
    }
  } catch {
    selectedAsset.value = null
    updateStatus.value = 'error'
    updateMessage.value = '检查失败，请稍后重试或打开 GitHub Release 页面'
  }
}

async function downloadUpdate() {
  if (downloadState.value === 'downloading' || !selectedAsset.value) {
    if (!selectedAsset.value && isHttpsUrl(releaseUrl.value)) window.open(releaseUrl.value, '_blank', 'noopener,noreferrer')
    return
  }
  downloadState.value = 'downloading'
  downloadProgress.value = 0
  downloadMessage.value = '正在准备下载…'
  try {
    const path = await invoke<string>('download_update', {
      url: selectedAsset.value.url,
      filename: selectedAsset.value.name,
      expectedSize: selectedAsset.value.size ?? null,
      expectedSha256: selectedAsset.value.sha256 ?? null
    })
    downloadState.value = 'completed'
    downloadProgress.value = 100
    downloadMessage.value = `下载完成：${path}`
  } catch (error) {
    downloadState.value = 'error'
    downloadMessage.value = error instanceof Error ? error.message : '下载失败，请稍后重试。'
  }
}

onMounted(async () => {
  const savedTheme = localStorage.getItem('warmdesk-theme') as Theme | null
  if (savedTheme === 'light' || savedTheme === 'dark' || savedTheme === 'system') {
    theme.value = savedTheme
  }
  colorScheme = window.matchMedia('(prefers-color-scheme: dark)')
  colorScheme.addEventListener('change', applyTheme)
  applyTheme()
  try {
    unlistenDownload = await listen<{
      downloaded_bytes: number
      total_bytes?: number
      percent?: number
    }>('download-progress', (event) => {
      const percent = event.payload.percent
      if (typeof percent === 'number') downloadProgress.value = percent
      if (event.payload.total_bytes) {
        downloadMessage.value = `已下载 ${(event.payload.downloaded_bytes / 1024 / 1024).toFixed(1)} / ${(event.payload.total_bytes / 1024 / 1024).toFixed(1)} MB`
      }
    })
  } catch {
    // Browser preview does not have the Tauri event bridge.
  }
  await Promise.all([loadUpdateInfo(), refreshStatus()])
  statusPoll = window.setInterval(refreshStatus, 2000)
})
onBeforeUnmount(() => {
  clearTimer()
  unlistenDownload?.()
  colorScheme?.removeEventListener('change', applyTheme)
})
</script>

<template>
  <main class="shell">
    <header class="topbar">
      <div class="brand">
        <div class="brand-mark"><Thermometer :size="18" /></div>
        <div>
        <strong>WarmDesk</strong>
          <span>低影响 CPU 暖手助手</span>
        </div>
      </div>
      <button class="icon-button" title="打开设置" @click="showSettings = !showSettings">
        <ChevronDown :size="18" :class="{ rotated: showSettings }" />
      </button>
    </header>

    <section class="hero">
      <div class="status-pill" :class="{ active: running }">
        <span class="status-dot"></span>
        {{ statusLabel }}
      </div>
      <div class="readout-row">
        <div class="temperature-wrap">
          <span class="temperature">{{ displayTemperature }}</span>
          <span class="temperature-caption">{{ temperatureCaption }}</span>
        </div>
        <div class="heat-ring" :style="{ '--heat': `${Math.max(6, intensity)}%` }">
          <div class="heat-core"><Thermometer :size="23" /></div>
        </div>
      </div>
      <p class="hero-copy">暖手强度是受控 CPU 时间预算，不是目标温度；实际热感取决于机型与环境。</p>
    </section>

    <div class="notice-strip" :class="{ warning: !desktopRuntimeAvailable || protectionReason }">
      <ShieldCheck :size="15" />
      <span>{{ commandError || safetyMessage }}</span>
    </div>

    <section class="telemetry-grid" aria-label="设备状态">
      <div class="telemetry-card"><Activity :size="15" /><span>CPU 负载</span><strong>{{ displayCpu }}</strong></div>
      <div class="telemetry-card"><Thermometer :size="15" /><span>温度</span><strong>{{ displayTemperature }}</strong></div>
      <div class="telemetry-card"><BatteryCharging :size="15" /><span>电源</span><strong>{{ powerLabel }}</strong></div>
      <div class="telemetry-card"><LockKeyhole :size="15" /><span>锁屏</span><strong>{{ lockStateAvailable ? locked ? '已锁屏' : '未锁屏' : '不可读取' }}</strong></div>
      <div class="telemetry-card"><ShieldCheck :size="15" /><span>热状态</span><strong>{{ thermalLabel }}</strong></div>
    </section>

    <section class="panel controls">
      <div class="panel-heading">
        <div>
          <span class="eyebrow">主要发热强度设定</span>
          <h2>{{ intensity }}<small>%</small></h2>
        </div>
        <span class="load-chip"><Activity :size="14" /> {{ intensity < 20 ? '微温柔和' : intensity < 40 ? '适中' : '偏暖' }}</span>
      </div>
      <input v-model.number="intensity" class="range" type="range" min="5" max="60" step="1" aria-label="暖手强度" @change="updateWarmSettings({ intensity })" />
      <div class="range-labels"><span>5% · 柔和</span><span>20%</span><span>40%</span><span>60% · 建议上限</span></div>
      <p class="helper">强度按整机逻辑 CPU 总容量换算，不同核心数的电脑会保持接近的整体占用。系统高负载、温度过高、电量过低或锁屏时会自动降载。</p>
    </section>

    <section class="panel mode-panel">
      <div class="panel-heading compact">
        <span class="eyebrow">工作模式</span>
        <span class="mode-name">{{ modeLabel }}</span>
      </div>
      <div class="segmented">
        <button :class="{ selected: mode === 'steady' }" @click="mode = 'steady'; updateWarmSettings({ mode: 'steady' })">恒温</button>
        <button :class="{ selected: mode === 'pulse' }" @click="mode = 'pulse'; updateWarmSettings({ mode: 'pulse' })">脉冲</button>
        <button :class="{ selected: mode === 'adaptive' }" @click="mode = 'adaptive'; updateWarmSettings({ mode: 'adaptive' })">自适应</button>
      </div>
    </section>

    <section class="session-row">
      <div class="session-stat">
        <Clock3 :size="17" />
        <div><span>本次暖手运行时间</span><strong>{{ elapsedLabel }} <small v-if="running">· {{ remainingLabel }}</small></strong></div>
      </div>
      <button class="text-button" title="重置本次运行" @click="resetSession"><TimerReset :size="16" /> 重置</button>
    </section>

    <section class="timer-options">
      <span class="eyebrow">自动停止</span>
      <div class="timer-buttons">
        <button v-for="option in [30, 60, 120, 0]" :key="option" :class="{ selected: duration === option }" @click="duration = option">
          {{ option === 0 ? '不限时' : `${option} 分钟` }}
        </button>
      </div>
    </section>

    <button class="primary-action" :class="{ running }" :disabled="busy || !desktopRuntimeAvailable" @click="toggleRunning">
      <Pause v-if="running" :size="19" />
      <Play v-else :size="19" />
      {{ busy ? '正在切换…' : running ? '停止暖手' : '开始暖手' }}
    </button>

    <div class="safety-note">
      <ShieldCheck :size="16" />
      <span>免责声明：使用本软件的全部风险由使用者承担；作者不承担因使用本软件产生的任何责任。真实暖手会消耗 CPU 与电量，请保持通风，异常时立即停止。</span>
    </div>

    <transition name="slide">
      <section v-if="showSettings" class="settings-panel">
        <div class="panel-heading compact"><span class="eyebrow">安全边界与外观</span><Power :size="16" /></div>
        <div class="setting-row"><span>真实 CPU 负载</span><strong>{{ desktopRuntimeAvailable ? '已接入' : '桌面端可用' }}</strong></div>
        <div class="setting-row"><span>电池保护</span><strong>{{ batteryPercent !== null && batteryPercent < 20 ? '低电量降载' : '自动保护' }}</strong></div>
        <div class="disclaimer-box">安全与免责：本软件仅通过受控 CPU 负载产生机身余热，不是加热器，不保证固定温度、暖手效果或适配所有设备。使用者须自行承担使用或无法使用本软件的一切风险，包括但不限于 CPU 占用、功耗、风扇噪声、电池消耗、设备损坏、数据丢失、性能影响及人身伤害；作者不承担任何责任。请保持通风，不要覆盖出风口，并在有人看护时使用。温度采样可能不可用，软件不能替代硬件温控；发现异常发热、异味、噪声或系统不稳定时，请立即停止并断开电源。</div>
        <div class="setting-row"><span>界面主题</span><div class="theme-buttons"><button :class="{ selected: theme === 'system' }" title="跟随系统" @click="setTheme('system')"><Moon :size="13" /></button><button :class="{ selected: theme === 'light' }" title="浅色" @click="setTheme('light')"><Sun :size="13" /></button><button :class="{ selected: theme === 'dark' }" title="深色" @click="setTheme('dark')"><Zap :size="13" /></button></div></div>
        <div class="update-section">
          <div class="update-heading">
            <div>
              <span class="eyebrow">软件更新</span>
              <strong>当前版本 {{ currentVersion }}</strong>
            </div>
            <RefreshCw :size="15" :class="{ spinning: updateStatus === 'checking' }" />
          </div>
          <p class="update-message" :class="`is-${updateStatus}`">{{ updateMessage }}</p>
          <div v-if="downloadState === 'downloading' || downloadState === 'completed' || downloadState === 'error'" class="download-status">
            <div class="download-status-row">
              <span>{{ downloadState === 'downloading' ? '下载进度' : downloadState === 'completed' ? '下载完成' : '下载失败' }}</span>
              <strong v-if="downloadState !== 'error'">{{ downloadProgress }}%</strong>
            </div>
            <div v-if="downloadState !== 'error'" class="download-progress" role="progressbar" :aria-valuenow="downloadProgress" aria-valuemin="0" aria-valuemax="100">
              <span :style="{ width: `${downloadProgress}%` }"></span>
            </div>
            <p class="download-detail">{{ downloadMessage }}</p>
          </div>
          <div class="update-actions">
            <button class="update-button" :disabled="updateStatus === 'checking'" @click="checkForUpdates">
              <RefreshCw :size="13" /> 检查更新
            </button>
            <button class="release-link" :disabled="downloadState === 'downloading'" @click="downloadUpdate">
              <RefreshCw v-if="downloadState === 'downloading'" :size="12" class="spinning" />
              <ExternalLink v-else :size="12" />
              {{ downloadState === 'downloading' ? `下载中 ${downloadProgress}%` : updateStatus === 'available' && latestVersion ? `下载 ${latestVersion}` : '打开 Release' }}
            </button>
          </div>
        </div>
      </section>
    </transition>
  </main>
</template>
