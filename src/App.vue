<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import {
  Activity,
  ChevronDown,
  Clock3,
  ExternalLink,
  Pause,
  Play,
  Power,
  RefreshCw,
  ShieldCheck,
  Thermometer,
  TimerReset
} from '@lucide/vue'

type Mode = 'steady' | 'pulse' | 'adaptive'

const running = ref(false)
const intensity = ref(28)
const mode = ref<Mode>('steady')
const duration = ref(0)
const elapsed = ref(0)
const showSettings = ref(false)
const currentVersion = ref('0.1.0')
const updateStatus = ref<'idle' | 'checking' | 'up-to-date' | 'available' | 'error'>('idle')
const updateMessage = ref('尚未检查')
const latestVersion = ref('')
const releaseUrl = ref('https://github.com/kokotao/warmdesk/releases/latest')
const releaseApiUrl = ref('https://api.github.com/repos/kokotao/warmdesk/releases/latest')
const mirrorManifestUrl = ref('https://note.lizipro.cn/warmdesk/releases/latest.json')
const downloadSource = ref<'mirror' | 'github'>('github')
let tick: number | undefined

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

function selectAsset<T extends { name?: string; urls?: { note?: string; github?: string }; browser_download_url?: string }>(
  assets: T[] | undefined,
  isMirrorAsset: boolean
) {
  const windows = /windows/i.test(navigator.userAgent)
  const candidates = assets?.filter((asset) => {
    const name = asset.name?.toLowerCase() || ''
    return windows ? name.endsWith('.msi') || name.endsWith('.exe') : name.endsWith('.dmg')
  }) ?? []
  const asset = candidates.find((item) => item.name?.toLowerCase().endsWith(windows ? '.msi' : '.dmg')) ?? candidates[0]
  const url = isMirrorAsset ? asset?.urls?.note : asset?.browser_download_url
  return isHttpsUrl(url) ? url : ''
}

const elapsedLabel = computed(() => {
  const hours = Math.floor(elapsed.value / 3600)
  const minutes = Math.floor((elapsed.value % 3600) / 60)
  const seconds = elapsed.value % 60
  return [hours, minutes, seconds].map((part) => String(part).padStart(2, '0')).join(':')
})

function clearTimer() {
  if (tick !== undefined) {
    window.clearInterval(tick)
    tick = undefined
  }
}

function toggleRunning() {
  running.value = !running.value
  clearTimer()
  if (running.value) {
    tick = window.setInterval(() => {
      elapsed.value += 1
      if (duration.value > 0 && elapsed.value >= duration.value * 60) {
        running.value = false
        clearTimer()
      }
    }, 1000)
  }
}

function resetSession() {
  running.value = false
  elapsed.value = 0
  clearTimer()
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
      mirror_manifest_url: string
      release_api_url: string
      release_page_url: string
    }>('get_update_info')
    currentVersion.value = info.current_version
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
  try {
    let releaseVersion = ''
    let resolvedReleaseUrl = releaseUrl.value
    let resolvedDownloadUrl = ''
    try {
      const mirrorResponse = await fetch(mirrorManifestUrl.value, { headers: { Accept: 'application/json' } })
      if (!mirrorResponse.ok) throw new Error(`HTTP ${mirrorResponse.status}`)
      const manifest = await mirrorResponse.json() as {
        version?: string
        tag?: string
        release_url?: string
        assets?: Array<{
          name?: string
          urls?: { note?: string; github?: string }
        }>
      }
      if (!manifest.version || !Array.isArray(manifest.assets)) throw new Error('镜像 manifest 无效')
      releaseVersion = manifest.tag || manifest.version
      resolvedReleaseUrl = manifest.release_url || resolvedReleaseUrl
      resolvedDownloadUrl = selectAsset(manifest.assets, true)
      if (!resolvedDownloadUrl) {
        const githubAsset = manifest.assets.find((asset) => isHttpsUrl(asset.urls?.github))
        resolvedDownloadUrl = githubAsset?.urls?.github || ''
      }
      downloadSource.value = resolvedDownloadUrl && manifest.assets.some((asset) => asset.urls?.note === resolvedDownloadUrl)
        ? 'mirror'
        : 'github'
    } catch {
      const response = await fetch(releaseApiUrl.value, {
        headers: { Accept: 'application/vnd.github+json' }
      })
      if (!response.ok) throw new Error(`HTTP ${response.status}`)
      const release = await response.json() as {
        tag_name?: string
        html_url?: string
        assets?: Array<{ browser_download_url?: string; name?: string }>
        prerelease?: boolean
        draft?: boolean
      }
      if (!release.tag_name || release.draft || release.prerelease) throw new Error('暂无稳定版本')
      releaseVersion = release.tag_name
      resolvedReleaseUrl = release.html_url || resolvedReleaseUrl
      resolvedDownloadUrl = selectAsset(release.assets, false)
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
    updateStatus.value = 'error'
    updateMessage.value = '检查失败，请稍后重试或打开 GitHub Release 页面'
  }
}

onMounted(loadUpdateInfo)
onBeforeUnmount(clearTimer)
</script>

<template>
  <main class="shell">
    <header class="topbar">
      <div class="brand">
        <div class="brand-mark"><Thermometer :size="18" /></div>
        <div>
        <strong>WarmDesk</strong>
          <span>交互原型 · 模拟预览</span>
        </div>
      </div>
      <button class="icon-button" title="打开设置" @click="showSettings = !showSettings">
        <ChevronDown :size="18" :class="{ rotated: showSettings }" />
      </button>
    </header>

    <section class="hero">
      <div class="status-pill" :class="{ active: running }">
        <span class="status-dot"></span>
        {{ running ? '状态 · 预览中' : '状态 · 待命' }}
      </div>
      <div class="readout-row">
        <div class="temperature-wrap">
          <span class="temperature">{{ intensity }}<sup>%</sup></span>
          <span class="temperature-caption">当前设定强度</span>
        </div>
        <div class="heat-ring" :style="{ '--heat': `${Math.max(6, intensity)}%` }">
          <div class="heat-core"><Thermometer :size="23" /></div>
        </div>
      </div>
      <p class="hero-copy">强度只作预算参考，不代表设备温度。</p>
    </section>

    <div class="notice-strip">
      <ShieldCheck :size="15" />
      <span>模拟交互原型：未运行实际 CPU 负荷计算，也不会产生真实物理发热。</span>
    </div>

    <section class="panel controls">
      <div class="panel-heading">
        <div>
          <span class="eyebrow">主要发热强度设定</span>
          <h2>{{ intensity }}<small>%</small></h2>
        </div>
        <span class="load-chip"><Activity :size="14" /> {{ intensity < 20 ? '微温柔和' : intensity < 40 ? '适中' : '偏暖' }}</span>
      </div>
      <input v-model.number="intensity" class="range" type="range" min="5" max="60" step="1" aria-label="暖手强度" />
      <div class="range-labels"><span>5% · 柔和</span><span>20%</span><span>40%</span><span>60% · 建议上限</span></div>
      <p class="helper">强度只代表未来工作预算，不是实际温度或 CPU 锁定值；当前未启动计算任务。</p>
    </section>

    <section class="panel mode-panel">
      <div class="panel-heading compact">
        <span class="eyebrow">工作模式</span>
        <span class="mode-name">{{ modeLabel }}</span>
      </div>
      <div class="segmented">
        <button :class="{ selected: mode === 'steady' }" @click="mode = 'steady'">恒温</button>
        <button :class="{ selected: mode === 'pulse' }" @click="mode = 'pulse'">脉冲</button>
        <button :class="{ selected: mode === 'adaptive' }" @click="mode = 'adaptive'">自适应</button>
      </div>
    </section>

    <section class="session-row">
      <div class="session-stat">
        <Clock3 :size="17" />
        <div><span>本次预览运行时间</span><strong>{{ elapsedLabel }}</strong></div>
      </div>
      <button class="text-button" title="重置本次运行" @click="resetSession"><TimerReset :size="16" /> 重置</button>
    </section>

    <section class="timer-options">
      <span class="eyebrow">自动停止</span>
      <div class="timer-buttons">
        <button v-for="option in [30, 60, 120]" :key="option" :class="{ selected: duration === option }" @click="duration = option">
          {{ option }} 分钟
        </button>
      </div>
    </section>

    <button class="primary-action" :class="{ running }" @click="toggleRunning">
      <Pause v-if="running" :size="19" />
      <Play v-else :size="19" />
      {{ running ? '停止预览' : '开始预览' }}
    </button>

    <div class="safety-note">
      <ShieldCheck :size="16" />
      <span>模拟运行状态，不会加热设备。真实热量必然消耗 CPU 与电量，且可能影响性能。</span>
    </div>

    <transition name="slide">
      <section v-if="showSettings" class="settings-panel">
        <div class="panel-heading compact"><span class="eyebrow">安全边界</span><Power :size="16" /></div>
        <div class="setting-row"><span>真实 CPU 负载</span><strong>未接入</strong></div>
        <div class="setting-row"><span>温度传感器</span><strong>不可用</strong></div>
        <div class="setting-row"><span>硬件/电源策略修改</span><strong>不支持</strong></div>
        <div class="update-section">
          <div class="update-heading">
            <div>
              <span class="eyebrow">软件更新</span>
              <strong>当前版本 {{ currentVersion }}</strong>
            </div>
            <RefreshCw :size="15" :class="{ spinning: updateStatus === 'checking' }" />
          </div>
          <p class="update-message" :class="`is-${updateStatus}`">{{ updateMessage }}</p>
          <div class="update-actions">
            <button class="update-button" :disabled="updateStatus === 'checking'" @click="checkForUpdates">
              <RefreshCw :size="13" /> 检查更新
            </button>
            <a class="release-link" :href="releaseUrl" target="_blank" rel="noreferrer">
              {{ updateStatus === 'available' && latestVersion ? `下载 ${latestVersion}` : '打开 Release' }}
              <ExternalLink :size="12" />
            </a>
          </div>
        </div>
      </section>
    </transition>
  </main>
</template>
