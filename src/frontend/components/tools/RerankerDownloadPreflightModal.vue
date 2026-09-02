<script setup lang="ts">
import type {
  DetectedRerankerProxy,
  RerankerDownloadProbeResult,
  RerankerDownloadSelection,
  RerankerProxySettings,
  RerankerRouteProbe,
} from '../../types/rerankerDownload'
import { invoke } from '@tauri-apps/api/core'
import { computed, ref, watch } from 'vue'

interface ManualProxySettings {
  proxy_type: RerankerProxySettings['proxy_type']
  host: string
  port: number | null
}

const props = defineProps<{
  show: boolean
  operating: boolean
}>()

const emit = defineEmits<{
  (event: 'update:show', value: boolean): void
  (event: 'confirm', value: RerankerDownloadSelection): void
}>()

const showModal = computed({
  get: () => props.show,
  set: value => emit('update:show', value),
})

const detecting = ref(false)
const detectionCompleted = ref(false)
const detectionError = ref('')
const detectedProxies = ref<DetectedRerankerProxy[]>([])
const selectedProxyKey = ref('')
const manualVisible = ref(false)
const routeMode = ref<'direct' | 'proxy'>('direct')
const rememberProxy = ref(false)
const probing = ref(false)
const probeError = ref('')
const probeResult = ref<RerankerDownloadProbeResult | null>(null)
const manualProxy = ref<ManualProxySettings>({
  proxy_type: 'http',
  host: '127.0.0.1',
  port: 7890,
})

const proxyTypeOptions = [
  { label: 'HTTP / HTTPS', value: 'http' },
  { label: 'SOCKS5', value: 'socks5' },
]

const selectedDetectedProxy = computed(() =>
  detectedProxies.value.find(proxy => proxyKey(proxy) === selectedProxyKey.value) || null,
)

const manualProxyValid = computed(() =>
  manualProxy.value.host.trim().length > 0
  && typeof manualProxy.value.port === 'number'
  && Number.isInteger(manualProxy.value.port)
  && manualProxy.value.port >= 1
  && manualProxy.value.port <= 65535,
)

const effectiveProxy = computed<RerankerProxySettings | null>(() => {
  if (manualVisible.value) {
    if (!manualProxyValid.value)
      return null
    const port = manualProxy.value.port
    if (port === null)
      return null
    return {
      proxy_type: manualProxy.value.proxy_type,
      host: manualProxy.value.host.trim(),
      port,
    }
  }
  const proxy = selectedDetectedProxy.value
  if (!proxy)
    return null
  return {
    proxy_type: proxy.proxy_type,
    host: proxy.host,
    port: proxy.port,
  }
})

const busy = computed(() => detecting.value || probing.value || props.operating)
const canConfirm = computed(() =>
  detectionCompleted.value
  && !busy.value
  && (routeMode.value === 'direct' || effectiveProxy.value !== null),
)
const canProbe = computed(() =>
  detectionCompleted.value
  && !busy.value
  && (routeMode.value === 'direct' || effectiveProxy.value !== null),
)

const probeRows = computed(() => {
  if (!probeResult.value)
    return []
  return [probeResult.value.direct, probeResult.value.proxy].filter(
    (value): value is RerankerRouteProbe => Boolean(value),
  )
})

const recommendedMode = computed<'direct' | 'proxy' | null>(() => {
  const direct = probeResult.value?.direct
  const proxy = probeResult.value?.proxy
  if (!direct?.available && !proxy?.available)
    return null
  if (!proxy?.available)
    return direct?.available ? 'direct' : null
  if (!direct?.available)
    return 'proxy'
  return (proxy.median_bytes_per_second || 0) > (direct.median_bytes_per_second || 0)
    ? 'proxy'
    : 'direct'
})

watch(() => props.show, (show) => {
  if (show)
    void initialize()
})

watch(routeMode, (mode) => {
  if (mode === 'direct')
    rememberProxy.value = false
})

function proxyKey(proxy: RerankerProxySettings): string {
  return `${proxy.proxy_type}:${proxy.host}:${proxy.port}`
}

function clearProbe() {
  probeResult.value = null
  probeError.value = ''
}

async function initialize() {
  detectionCompleted.value = false
  detectionError.value = ''
  detectedProxies.value = []
  selectedProxyKey.value = ''
  manualVisible.value = false
  routeMode.value = 'direct'
  rememberProxy.value = false
  clearProbe()
  await detectProxies()
}

async function detectProxies() {
  // 中文说明：弹窗打开后先复用现有端口探测能力，避免用户盲填常见代理端口。
  detecting.value = true
  detectionCompleted.value = false
  detectionError.value = ''
  detectedProxies.value = []
  selectedProxyKey.value = ''
  clearProbe()
  try {
    const proxies = await invoke<DetectedRerankerProxy[]>('detect_acemcp_proxy', {
      extraPorts: [],
    })
    detectedProxies.value = proxies
    if (proxies.length > 0) {
      selectDetectedProxy(proxyKey(proxies[0]))
      manualVisible.value = false
    }
    else {
      manualVisible.value = true
      routeMode.value = 'direct'
    }
  }
  catch (error) {
    detectionError.value = String(error)
    manualVisible.value = true
    routeMode.value = 'direct'
  }
  finally {
    detectionCompleted.value = true
    detecting.value = false
  }
}

function selectDetectedProxy(key: string) {
  selectedProxyKey.value = key
  manualVisible.value = false
  routeMode.value = 'proxy'
  clearProbe()
}

function showManualProxy() {
  const selected = selectedDetectedProxy.value
  if (selected) {
    manualProxy.value = {
      proxy_type: selected.proxy_type,
      host: selected.host,
      port: selected.port,
    }
  }
  manualVisible.value = true
  routeMode.value = 'proxy'
  clearProbe()
}

function useDetectedProxy() {
  if (!selectedDetectedProxy.value && detectedProxies.value[0])
    selectedProxyKey.value = proxyKey(detectedProxies.value[0])
  manualVisible.value = false
  routeMode.value = 'proxy'
  clearProbe()
}

async function runProbe() {
  // 中文说明：测速只读取固定模型的 Range 小样本，不写入模型目录，也不改变全局代理。
  if (!canProbe.value)
    return
  probing.value = true
  probeError.value = ''
  probeResult.value = null
  try {
    const result = await invoke<RerankerDownloadProbeResult>('probe_sou_reranker_download_routes', {
      proxy: effectiveProxy.value,
    })
    probeResult.value = result
    const directSpeed = result.direct.available ? result.direct.median_bytes_per_second || 0 : 0
    const proxySpeed = result.proxy?.available ? result.proxy.median_bytes_per_second || 0 : 0
    if (proxySpeed > directSpeed)
      routeMode.value = 'proxy'
    else if (directSpeed > 0)
      routeMode.value = 'direct'
  }
  catch (error) {
    probeError.value = String(error)
  }
  finally {
    probing.value = false
  }
}

function confirmDownload() {
  if (!canConfirm.value)
    return
  if (routeMode.value === 'proxy' && effectiveProxy.value) {
    emit('confirm', {
      network: { mode: 'proxy', proxy: effectiveProxy.value },
      remember_proxy: rememberProxy.value,
    })
    return
  }
  emit('confirm', {
    network: { mode: 'direct' },
    remember_proxy: false,
  })
}

function probeTagType(probe: RerankerRouteProbe): 'success' | 'warning' | 'error' {
  if (!probe.available)
    return 'error'
  if (probe.completed_samples < probe.total_samples || !probe.supports_ranges)
    return 'warning'
  return 'success'
}

function formatSpeed(value: number | null): string {
  if (!value || !Number.isFinite(value) || value <= 0)
    return '--'
  if (value >= 1024 * 1024)
    return `${(value / 1024 / 1024).toFixed(2)} MiB/s`
  return `${(value / 1024).toFixed(1)} KiB/s`
}

function formatSpeedRange(probe: RerankerRouteProbe): string {
  if (!probe.min_bytes_per_second || !probe.max_bytes_per_second)
    return '--'
  return `${formatSpeed(probe.min_bytes_per_second)} - ${formatSpeed(probe.max_bytes_per_second)}`
}
</script>

<template>
  <n-modal
    v-model:show="showModal"
    preset="card"
    title="下载前网络检查"
    :style="{ width: '680px', maxWidth: 'calc(100vw - 32px)' }"
    :bordered="false"
    :closable="!busy"
    :mask-closable="!busy"
    :close-on-esc="!busy"
  >
    <div class="preflight-heading">
      <div class="i-carbon-network-3 preflight-heading-icon text-primary-500" aria-hidden="true" />
      <div class="min-w-0">
        <div class="font-medium">
          BAAI/bge-reranker-base
        </div>
        <div class="form-feedback">
          本次选择默认不修改全局代理；测速仅读取固定模型的小段数据。
        </div>
      </div>
    </div>

    <n-divider />

    <section class="preflight-section">
      <div class="section-heading">
        <div>
          <div class="section-title">
            本地代理
          </div>
          <div class="form-feedback">
            自动检查 Clash、V2Ray 等常用端口
          </div>
        </div>
        <n-button secondary size="small" :loading="detecting" :disabled="props.operating" @click="detectProxies">
          <template #icon>
            <div class="i-carbon-renew" />
          </template>
          重新检测
        </n-button>
      </div>

      <n-spin :show="detecting">
        <div v-if="detectedProxies.length > 0" class="proxy-list">
          <n-radio-group :value="selectedProxyKey" @update:value="selectDetectedProxy">
            <n-space vertical size="small">
              <n-radio v-for="proxy in detectedProxies" :key="proxyKey(proxy)" :value="proxyKey(proxy)">
                <span class="proxy-option">
                  <strong>{{ proxy.proxy_type.toUpperCase() }}</strong>
                  <span>{{ proxy.host }}:{{ proxy.port }}</span>
                  <span class="form-feedback">{{ proxy.response_time_ms ?? '--' }} ms</span>
                </span>
              </n-radio>
            </n-space>
          </n-radio-group>
        </div>

        <n-alert v-else-if="detectionCompleted && !detectionError" type="warning" :bordered="false">
          未检测到常用本地代理端口。可以手动填写代理，或选择直连。
        </n-alert>
        <n-alert v-if="detectionError" type="warning" :bordered="false">
          自动检测失败：{{ detectionError }}。可以手动填写代理，或选择直连。
        </n-alert>
      </n-spin>

      <div class="manual-actions">
        <n-button v-if="!manualVisible" text type="primary" @click="showManualProxy">
          <template #icon>
            <div class="i-carbon-edit" />
          </template>
          手动填写代理
        </n-button>
        <n-button v-else-if="detectedProxies.length > 0" text type="primary" @click="useDetectedProxy">
          使用检测结果
        </n-button>
      </div>

      <div v-if="manualVisible" class="manual-grid">
        <label class="field-block">
          <span class="field-label">类型</span>
          <n-select
            v-model:value="manualProxy.proxy_type"
            :options="proxyTypeOptions"
            @update:value="clearProbe"
          />
        </label>
        <label class="field-block field-host">
          <span class="field-label">地址</span>
          <n-input
            v-model:value="manualProxy.host"
            placeholder="127.0.0.1"
            clearable
            @focus="routeMode = 'proxy'"
            @update:value="clearProbe"
          />
        </label>
        <label class="field-block">
          <span class="field-label">端口</span>
          <n-input-number
            v-model:value="manualProxy.port"
            :min="1"
            :max="65535"
            :show-button="false"
            class="w-full"
            @focus="routeMode = 'proxy'"
            @update:value="clearProbe"
          />
        </label>
      </div>
    </section>

    <n-divider />

    <section class="preflight-section">
      <div class="section-heading route-heading">
        <div>
          <div class="section-title">
            下载路线
          </div>
          <div class="form-feedback">
            代理不可用时，下载器仍会按既有顺序回退
          </div>
        </div>
        <n-radio-group v-model:value="routeMode" name="reranker-download-route">
          <n-radio-button value="proxy" :disabled="effectiveProxy === null">
            本地代理
          </n-radio-button>
          <n-radio-button value="direct">
            直连
          </n-radio-button>
        </n-radio-group>
      </div>
      <n-checkbox v-if="routeMode === 'proxy'" v-model:checked="rememberProxy">
        记住为全局下载代理
      </n-checkbox>
    </section>

    <n-divider />

    <section class="preflight-section">
      <div class="section-heading">
        <div>
          <div class="section-title">
            下载测速
          </div>
          <div class="form-feedback">
            每条路线最多读取 3 个 512 KiB Range 样本
          </div>
        </div>
        <n-button secondary size="small" :loading="probing" :disabled="!canProbe" @click="runProbe">
          <template #icon>
            <div class="i-carbon-chart-line" />
          </template>
          {{ effectiveProxy ? '对比测速' : '测试直连' }}
        </n-button>
      </div>

      <n-alert v-if="probeError" type="error" :bordered="false">
        测速失败：{{ probeError }}
      </n-alert>

      <div v-if="probeRows.length > 0" class="probe-table">
        <div v-for="probe in probeRows" :key="probe.mode" class="probe-row">
          <div class="probe-route">
            <div class="probe-route-title">
              <span>{{ probe.label }}</span>
              <n-tag v-if="recommendedMode === probe.mode" type="success" size="small" :bordered="false">
                推荐
              </n-tag>
              <n-tag :type="probeTagType(probe)" size="small" :bordered="false">
                {{ probe.available ? `成功 ${probe.completed_samples}/${probe.total_samples}` : '不可用' }}
              </n-tag>
            </div>
            <div class="probe-metrics">
              <span>中位 {{ formatSpeed(probe.median_bytes_per_second) }}</span>
              <span>区间 {{ formatSpeedRange(probe) }}</span>
              <span>波动 {{ probe.variation_percent == null ? '--' : `${probe.variation_percent.toFixed(0)}%` }}</span>
              <span>首字节 {{ probe.median_ttfb_ms == null ? '--' : `${probe.median_ttfb_ms} ms` }}</span>
              <span>{{ probe.supports_ranges ? 'Range 支持' : 'Range 未确认' }}</span>
            </div>
            <div v-if="probe.error" class="probe-error text-error">
              {{ probe.error }}
            </div>
          </div>
        </div>
      </div>
    </section>

    <template #footer>
      <div class="modal-footer">
        <n-button :disabled="busy" @click="showModal = false">
          取消
        </n-button>
        <n-button type="primary" :loading="props.operating" :disabled="!canConfirm" @click="confirmDownload">
          <template #icon>
            <div class="i-carbon-download" />
          </template>
          使用所选路线下载
        </n-button>
      </div>
    </template>
  </n-modal>
</template>

<style scoped>
.preflight-heading,
.section-heading,
.probe-route-title,
.modal-footer {
  display: flex;
  align-items: center;
}

.preflight-heading {
  gap: 12px;
}

.preflight-heading-icon {
  width: 24px;
  height: 24px;
  flex: 0 0 auto;
}

.preflight-section {
  display: flex;
  flex-direction: column;
  gap: 12px;
}

.section-heading {
  justify-content: space-between;
  gap: 16px;
}

.section-title {
  color: var(--color-on-surface, #e5e7eb);
  font-size: 13px;
  font-weight: 600;
}

.form-feedback {
  overflow-wrap: anywhere;
  color: var(--color-on-surface-muted, #9ca3af);
  font-size: 11px;
}

.proxy-list {
  padding: 2px 0;
}

.proxy-option {
  display: inline-flex;
  align-items: center;
  flex-wrap: wrap;
  gap: 8px;
  min-width: 0;
}

.manual-actions {
  min-height: 22px;
}

.manual-grid {
  display: grid;
  grid-template-columns: minmax(130px, 0.8fr) minmax(180px, 1.5fr) minmax(110px, 0.7fr);
  gap: 12px;
}

.field-block {
  display: flex;
  min-width: 0;
  flex-direction: column;
  gap: 6px;
}

.field-label {
  color: var(--color-on-surface-muted, #9ca3af);
  font-size: 11px;
}

.probe-table {
  border-top: 1px solid var(--color-border, rgba(128, 128, 128, 0.2));
}

.probe-row {
  padding: 12px 0;
  border-bottom: 1px solid var(--color-border, rgba(128, 128, 128, 0.2));
}

.probe-route {
  min-width: 0;
}

.probe-route-title {
  flex-wrap: wrap;
  gap: 8px;
  color: var(--color-on-surface, #e5e7eb);
  font-size: 12px;
  font-weight: 600;
}

.probe-metrics {
  display: flex;
  margin-top: 6px;
  flex-wrap: wrap;
  gap: 6px 14px;
  color: var(--color-on-surface-muted, #9ca3af);
  font-size: 11px;
}

.probe-error {
  margin-top: 6px;
  overflow-wrap: anywhere;
  font-size: 11px;
}

.modal-footer {
  justify-content: flex-end;
  flex-wrap: wrap;
  gap: 10px;
}

@media (max-width: 620px) {
  .manual-grid {
    grid-template-columns: 1fr;
  }

  .section-heading,
  .route-heading {
    align-items: flex-start;
    flex-direction: column;
  }
}
</style>
