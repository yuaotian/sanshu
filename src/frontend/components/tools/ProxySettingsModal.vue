<!-- eslint-disable vue/no-mutating-props -->
<!-- eslint-disable style/max-statements-per-line -->
<script setup lang="ts">
/**
 * 代理设置独立弹窗组件
 * 包含：代理配置、自动检测、测速、测速报告等功能
 */
import { invoke } from '@tauri-apps/api/core'
import { useDialog, useMessage } from 'naive-ui'
import { computed, ref, watch } from 'vue'

// Props
const props = defineProps<{
  show: boolean
  config: any // 传入的 config 对象（响应式）
}>()

const emit = defineEmits<{
  (e: 'update:show', v: boolean): void
}>()

const message = useMessage()
const dialog = useDialog()

// --- 类型定义 ---
interface DetectedProxy {
  host: string
  port: number
  proxy_type: string
  response_time_ms: number | null
}

interface SpeedTestMetric {
  name: string
  metric_type: string
  proxy_time_ms: number | null
  direct_time_ms: number | null
  success: boolean
  error: string | null
}

interface SpeedTestResult {
  mode: string
  proxy_info: DetectedProxy | null
  metrics: SpeedTestMetric[]
  timestamp: string
  recommendation: string
  success: boolean
}

interface SpeedTestQueryDetail {
  query: string
  proxy_time_ms: number | null
  direct_time_ms: number | null
  success: boolean
  error: string | null
}

type IndexStatus = 'idle' | 'indexing' | 'synced' | 'failed'

interface ProjectIndexStatusLite {
  project_root: string
  status: IndexStatus
  total_files: number
  last_success_time: string | null
}

// --- 状态变量 ---

const proxyDetecting = ref(false)
const detectedProxies = ref<DetectedProxy[]>([])
const proxyTesting = ref(false)
const speedTestResult = ref<SpeedTestResult | null>(null)
const speedTestProgress = ref('')
const speedTestMode = ref<'proxy' | 'direct' | 'compare'>('compare')
const speedTestQuery = ref('代码搜索测试')
const multiQuerySearchDetails = ref<SpeedTestQueryDetail[]>([])
const multiQueryDetailsExpanded = ref(false)

const extraDetectPortsText = ref('')
const proxyPickerVisible = ref(false)
const selectedProxyIndex = ref(0)

// 测速项目相关
const speedTestProjectRoot = ref('')
const projectPickerVisible = ref(false)
const projectPickerLoading = ref(false)
const projectPickerSelected = ref('')
const indexedProjects = ref<ProjectIndexStatusLite[]>([])
const projectUploadMode = ref<'sample' | 'full'>('sample')
const projectUploadMaxFiles = ref(200)

const addProjectVisible = ref(false)
const addProjectPath = ref('')
const addProjectIndexing = ref(false)

// --- 计算属性 ---

const showModal = computed({
  get: () => props.show,
  set: v => emit('update:show', v),
})

const speedTestQueries = computed(() => {
  return (speedTestQuery.value || '')
    .split(/\r?\n/g)
    .flatMap(line => line.split(';'))
    .map(s => s.trim())
    .filter(Boolean)
    .slice(0, 5)
})

const multiQuerySearchSummary = computed(() => {
  const list = multiQuerySearchDetails.value
  if (list.length <= 1) {
    return null
  }

  const proxyTimes = list.map(i => i.proxy_time_ms).filter((v): v is number => v !== null)
  const directTimes = list.map(i => i.direct_time_ms).filter((v): v is number => v !== null)

  const proxyAvg = proxyTimes.length > 0
    ? Math.round(proxyTimes.reduce((a, b) => a + b, 0) / proxyTimes.length)
    : null

  const directAvg = directTimes.length > 0
    ? Math.round(directTimes.reduce((a, b) => a + b, 0) / directTimes.length)
    : null

  return {
    total: list.length,
    proxy_avg_ms: proxyAvg,
    direct_avg_ms: directAvg,
    proxy_ok: proxyTimes.length,
    direct_ok: directTimes.length,
  }
})

const speedTestMetricsForDisplay = computed(() => {
  const r = speedTestResult.value
  if (!r) {
    return []
  }

  const metrics = r.metrics || []

  // 多查询时：逐条搜索指标会比较多，默认只展示“搜索平均 + 其他指标”
  if (multiQuerySearchSummary.value) {
    const out = metrics.filter(m => m.metric_type !== 'search')

    // 兜底：如果没有“搜索平均”，保留第一条搜索指标
    if (!out.some(m => m.metric_type === 'search_multi_avg')) {
      const firstSearch = metrics.find(m => m.metric_type === 'search')
      if (firstSearch) {
        out.push(firstSearch)
      }
    }

    return out
  }

  return metrics
})

// 测速按钮禁用逻辑
const speedTestDisabled = computed(() => {
  if (!props.config.base_url || !props.config.token) {
    return true
  }
  if (speedTestMode.value === 'direct') {
    return false
  }
  return !props.config.proxy_host || !props.config.proxy_port
})

// 测速按钮禁用原因
const speedTestDisabledReason = computed(() => {
  if (!props.config.base_url) {
    return '请先配置租户地址'
  }
  if (!props.config.token) {
    return '请先配置 ACE Token'
  }
  if (speedTestMode.value === 'direct') {
    return ''
  }
  if (!props.config.proxy_host) {
    return '请先填写代理地址（或使用自动检测）'
  }
  if (!props.config.proxy_port) {
    return '请先填写代理端口'
  }
  return ''
})

// --- 方法 ---

/** 自动检测本地代理 */
async function detectProxy() {
  proxyDetecting.value = true
  detectedProxies.value = []
  try {
    const extraPorts = parseExtraPorts(extraDetectPortsText.value)
    const proxies = await invoke('detect_acemcp_proxy', {
      extraPorts,
    }) as DetectedProxy[]
    detectedProxies.value = proxies

    if (proxies.length === 0) {
      message.warning('未检测到本地代理，请手动输入')
    }
    else if (proxies.length === 1) {
      applyProxy(proxies[0])
      message.success(`已检测到代理 ${proxies[0].host}:${proxies[0].port}，建议测速验证`)
    }
    else {
      selectedProxyIndex.value = 0
      proxyPickerVisible.value = true
      message.success(`检测到 ${proxies.length} 个代理，请选择一个`)
    }
  }
  catch (err) {
    message.error(`代理检测失败: ${err}`)
  }
  finally {
    proxyDetecting.value = false
  }
}

function parseExtraPorts(input: string): number[] {
  const parts = (input || '')
    .split(/[,，\s]+/g)
    .map(s => s.trim())
    .filter(Boolean)

  const nums = parts
    .map(s => Number(s))
    .filter(n => Number.isInteger(n) && n >= 1 && n <= 65535)

  return Array.from(new Set(nums))
}

function applyProxy(p: DetectedProxy) {
  props.config.proxy_host = p.host
  props.config.proxy_port = p.port
  props.config.proxy_type = p.proxy_type as 'http' | 'https' | 'socks5'
}

function confirmProxySelection() {
  const p = detectedProxies.value[selectedProxyIndex.value]
  if (!p) {
    message.warning('请先选择一个代理')
    return
  }
  applyProxy(p)
  proxyPickerVisible.value = false
  message.success(`已选择代理 ${p.host}:${p.port}`)
}

async function loadIndexedProjectsForSpeedTest() {
  projectPickerLoading.value = true
  try {
    const statusResult = await invoke<{ projects: Record<string, ProjectIndexStatusLite> }>('get_all_acemcp_index_status')
    const list = Object.values(statusResult.projects || {})
      .filter(p => (p.total_files || 0) > 0)

    indexedProjects.value = list
  }
  catch (e) {
    message.error(`加载已索引项目失败: ${e}`)
    indexedProjects.value = []
  }
  finally {
    projectPickerLoading.value = false
  }
}

async function openProjectPicker() {
  await loadIndexedProjectsForSpeedTest()

  if (indexedProjects.value.length === 0) {
    dialog.warning({
      title: '需要索引项目',
      content: '测速功能需要至少一个已索引的项目。是否现在添加项目并开始索引？',
      positiveText: '是',
      negativeText: '否',
      onPositiveClick: () => {
        addProjectVisible.value = true
      },
    })
    return
  }

  projectPickerSelected.value = speedTestProjectRoot.value || indexedProjects.value[0].project_root

  // 强制确保有选中值，如果当前没有，则选中列表第一个
  if (!projectPickerSelected.value && indexedProjects.value.length > 0) {
    projectPickerSelected.value = indexedProjects.value[0].project_root
  }

  projectPickerVisible.value = true
}

async function confirmProjectSelectionAndRun() {
  if (!projectPickerSelected.value) {
    message.warning('请选择一个测试项目')
    return
  }

  speedTestProjectRoot.value = projectPickerSelected.value
  projectPickerVisible.value = false

  await runSpeedTest()
}

async function addProjectAndIndexAndRun() {
  const path = addProjectPath.value.trim()
  if (!path) {
    message.error('请输入项目根路径')
    return
  }

  addProjectIndexing.value = true
  try {
    const exists = await invoke<boolean>('check_directory_exists', {
      directoryPath: path,
    })

    if (!exists) {
      message.error('目录不存在或不可访问，请检查路径')
      return
    }

    await invoke<string>('trigger_acemcp_index_update', {
      projectRootPath: path,
    })

    message.success('索引完成')
    speedTestProjectRoot.value = path
    addProjectVisible.value = false
    addProjectPath.value = ''

    await runSpeedTest()
  }
  catch (e) {
    message.error(`索引失败: ${e}`)
  }
  finally {
    addProjectIndexing.value = false
  }
}

async function runSpeedTest() {
  // Config 校验
  if (!props.config.base_url) {
    message.error('请先配置租户地址')
    return
  }
  if (!props.config.token) {
    message.error('请先配置 ACE Token')
    return
  }
  if (!speedTestProjectRoot.value) {
    await openProjectPicker()
    return
  }

  proxyTesting.value = true
  speedTestResult.value = null
  speedTestProgress.value = '正在准备测速...'
  multiQuerySearchDetails.value = []
  multiQueryDetailsExpanded.value = false

  try {
    const rawQueryCount = (speedTestQuery.value || '')
      .split(/\r?\n/g)
      .flatMap(line => line.split(';'))
      .map(s => s.trim())
      .filter(Boolean)
      .length

    if (rawQueryCount > 5) {
      message.info('测试查询过多，已按前 5 条执行')
    }

    const uploadMaxFiles = projectUploadMode.value === 'sample'
      ? Math.max(1, Number(projectUploadMaxFiles.value) || 200)
      : undefined

    const effectiveTestQuery = (speedTestQuery.value || '').trim()
      ? speedTestQuery.value
      : '代码搜索测试'

    const result = await invoke('test_acemcp_proxy_speed', {
      testMode: speedTestMode.value,
      proxyHost: props.config.proxy_host,
      proxyPort: props.config.proxy_port,
      proxyType: props.config.proxy_type,
      proxyUsername: props.config.proxy_username,
      proxyPassword: props.config.proxy_password,
      testQuery: effectiveTestQuery,
      projectRootPath: speedTestProjectRoot.value,
      projectUploadMode: projectUploadMode.value,
      projectUploadMaxFiles: uploadMaxFiles,
    }) as SpeedTestResult

    const effectiveQueries = speedTestQueries.value.length > 0
      ? speedTestQueries.value
      : ['代码搜索测试']

    const searchMetrics = (result.metrics || []).filter(m => m.metric_type === 'search')
    multiQuerySearchDetails.value = effectiveQueries.map((q, idx) => {
      const m = searchMetrics[idx]
      return {
        query: q,
        proxy_time_ms: m?.proxy_time_ms ?? null,
        direct_time_ms: m?.direct_time_ms ?? null,
        success: m?.success ?? false,
        error: m?.error ?? (m ? null : '未返回搜索指标'),
      }
    })

    const s = multiQuerySearchSummary.value
    if (s) {
      const avgMetric: SpeedTestMetric = {
        name: `🔎 语义搜索（${s.total} 条平均）`,
        metric_type: 'search_multi_avg',
        proxy_time_ms: s.proxy_avg_ms,
        direct_time_ms: s.direct_avg_ms,
        success: true,
        error: null,
      }

      if (speedTestMode.value !== 'direct' && s.proxy_ok === 0) {
        avgMetric.success = false
        avgMetric.error = '代理侧无有效搜索耗时（全部失败或未返回）'
      }
      if (speedTestMode.value !== 'proxy' && s.direct_ok === 0) {
        avgMetric.success = false
        avgMetric.error = [avgMetric.error, '直连侧无有效搜索耗时（全部失败或未返回）'].filter(Boolean).join('；')
      }

      result.metrics.push(avgMetric)
    }

    speedTestResult.value = result

    if (result.success) {
      message.success('测速完成')
    }
    else {
      message.warning('测速完成，部分测试失败')
    }
  }
  catch (err) {
    message.error(`测速失败: ${err}`)
  }
  finally {
    proxyTesting.value = false
    speedTestProgress.value = ''
  }
}

function buildSpeedTestReportPayload() {
  if (!speedTestResult.value) {
    return null
  }

  const uploadMaxFiles = projectUploadMode.value === 'sample'
    ? Math.max(1, Number(projectUploadMaxFiles.value) || 200)
    : undefined

  return {
    tool: 'sou',
    timestamp: speedTestResult.value.timestamp,
    mode: speedTestResult.value.mode,
    query: speedTestQuery.value,
    project: {
      root: speedTestProjectRoot.value,
      name: getProjectName(speedTestProjectRoot.value),
      upload_mode: projectUploadMode.value,
      upload_max_files: uploadMaxFiles,
    },
    proxy: speedTestResult.value.mode === 'direct'
      ? { enabled: false }
      : {
          enabled: true,
          type: props.config.proxy_type,
          host: props.config.proxy_host,
          port: props.config.proxy_port,
          username: props.config.proxy_username || undefined,
          password_set: Boolean(props.config.proxy_password),
        },
    config: {
      base_url: props.config.base_url,
      token_set: Boolean(props.config.token),
    },
    result: speedTestResult.value,
  }
}

async function copySpeedTestReport() {
  const report = buildSpeedTestReportPayload()
  if (!report) {
    message.warning('暂无测速结果可复制')
    return
  }

  try {
    await navigator.clipboard.writeText(JSON.stringify(report, null, 2))
    message.success('已复制测速报告（JSON）')
  }
  catch (e) {
    message.error(`复制失败: ${e}`)
  }
}

async function copyQueryDetail(detail: SpeedTestQueryDetail, idx: number) {
  if (!speedTestResult.value) {
    message.warning('暂无测速结果可复制')
    return
  }
  // 构造简略 payload
  const payload = {
    query: detail.query,
    proxy_ms: detail.proxy_time_ms,
    direct_ms: detail.direct_time_ms,
    success: detail.success,
    error: detail.error,
  }
  try {
    await navigator.clipboard.writeText(JSON.stringify(payload, null, 2))
    message.success(`已复制 Q${idx + 1} 明细`)
  }
  catch (e) {
    message.error(`复制失败: ${e}`)
  }
}

async function copyMetricResult(metric: SpeedTestMetric) {
  try {
    await navigator.clipboard.writeText(JSON.stringify(metric, null, 2))
    message.success(`已复制指标 "${metric.name}" 结果`)
  }
  catch (e) {
    message.error(`复制失败: ${e}`)
  }
}

async function downloadSpeedTestReport() {
  const report = buildSpeedTestReportPayload()
  if (!report) {
    message.warning('暂无测速结果可导出')
    return
  }

  try {
    const ts = speedTestResult.value?.timestamp || new Date().toISOString()
    const safeTs = ts.replace(/[:.]/g, '-').replace('T', '_').replace('Z', '')
    const filename = `sou-speedtest-${safeTs}.json`

    const blob = new Blob([JSON.stringify(report, null, 2)], { type: 'application/json;charset=utf-8' })
    const url = URL.createObjectURL(blob)

    const a = document.createElement('a')
    a.href = url
    a.download = filename
    a.click()

    setTimeout(() => URL.revokeObjectURL(url), 0)
    message.success(`已导出测速报告: ${filename}`)
  }
  catch (e) {
    message.error(`导出失败: ${e}`)
  }
}

// 辅助函数
function getProjectName(projectRoot: string): string {
  const parts = (projectRoot || '').replace(/\\/g, '/').split('/').filter(Boolean)
  return parts.length > 0 ? parts[parts.length - 1] : projectRoot
}

function formatIndexTime(ts: string | null): string {
  if (!ts)
    return '未完成'
  try { return new Date(ts).toLocaleString() }
  catch { return ts }
}

function formatSpeedTestTime(ts: string): string {
  if (!ts)
    return '-'
  try { return new Date(ts).toLocaleString() }
  catch { return ts }
}

function calcDiff(proxyMs: number | null, directMs: number | null): string {
  if (proxyMs === null || directMs === null)
    return '-'
  if (directMs === 0)
    return '-'
  const diff = ((directMs - proxyMs) / directMs * 100).toFixed(0)
  if (Number(diff) > 0)
    return `⬇️${diff}%`
  if (Number(diff) < 0)
    return `⬆️${Math.abs(Number(diff))}%`
  return '0%'
}

function getDiffColor(proxyMs: number | null, directMs: number | null): string {
  if (proxyMs === null || directMs === null)
    return 'inherit'
  if (proxyMs < directMs)
    return '#22c55e'
  if (proxyMs > directMs)
    return '#ef4444'
  return 'inherit'
}
</script>

<template>
  <n-modal
    v-model:show="showModal"
    class="custom-modal"
    preset="card"
    title="代理设置与网络诊断"
    :style="{ width: '800px', maxWidth: '95vw' }"
    :bordered="false"
    size="medium"
    role="dialog"
    aria-modal="true"
  >
    <div class="modal-content-wrapper">
      <!-- 顶部状态栏 -->
      <div class="mb-5 p-4 rounded-xl bg-gradient-to-r from-slate-50 to-slate-100 dark:from-slate-800 dark:to-slate-900/50 border border-slate-200 dark:border-slate-700 flex items-center justify-between">
        <div class="flex items-center gap-4">
          <div class="p-2 rounded-lg bg-blue-500/10 text-blue-600 dark:text-blue-400">
            <div class="i-carbon-network-3 text-2xl" />
          </div>
          <div>
            <div class="font-medium text-base mb-1">
              启用代理服务
            </div>
            <div class="text-xs text-gray-500 dark:text-gray-400">
              启用后，所有 ACE API 请求将通过此代理。
              <div v-if="!config.proxy_enabled" class="inline-block mt-1 px-1.5 py-0.5 rounded bg-orange-50 dark:bg-orange-900/30 text-orange-600 dark:text-orange-400 text-[10px]">
                当前直接连接
              </div>
              <div v-else class="inline-block mt-1 px-1.5 py-0.5 rounded bg-green-50 dark:bg-green-900/30 text-green-600 dark:text-green-400 text-[10px]">
                代理已启用 ({{ config.proxy_type.toUpperCase() }}://{{ config.proxy_host }}:{{ config.proxy_port }})
              </div>
            </div>
          </div>
        </div>
        <n-switch v-model:value="config.proxy_enabled" size="large">
          <template #checked>
            开启
          </template>
          <template #unchecked>
            关闭
          </template>
        </n-switch>
      </div>

      <n-tabs type="segment" animated>
        <!-- Tab 1: 代理配置 -->
        <n-tab-pane name="config" tab="配置参数">
          <n-space vertical size="large" class="pt-2">
            <!-- 代理表单 -->
            <div class="grid grid-cols-1 md:grid-cols-2 gap-4">
              <!-- 基础信息 -->
              <div class="md:col-span-2">
                <div class="text-xs font-semibold text-gray-500 uppercase tracking-wider mb-2">
                  基础连接
                </div>
                <div class="grid grid-cols-12 gap-3">
                  <div class="col-span-12 md:col-span-5">
                    <n-form-item label="Host (地址)" size="small">
                      <n-input v-model:value="config.proxy_host" placeholder="127.0.0.1" clearable />
                    </n-form-item>
                  </div>
                  <div class="col-span-12 md:col-span-3">
                    <n-form-item label="Port (端口)" size="small">
                      <n-input-number v-model:value="config.proxy_port" :min="1" :max="65535" class="w-full" :show-button="false" />
                    </n-form-item>
                  </div>
                  <div class="col-span-12 md:col-span-4">
                    <n-form-item label="Type (类型)" size="small">
                      <n-select v-model:value="config.proxy_type" :options="[{ label: 'HTTP', value: 'http' }, { label: 'HTTPS', value: 'https' }, { label: 'SOCKS5', value: 'socks5' }]" />
                    </n-form-item>
                  </div>
                </div>
              </div>

              <!-- 认证信息 -->
              <div class="md:col-span-2">
                <div class="text-xs font-semibold text-gray-500 uppercase tracking-wider mb-2 mt-2">
                  身份认证 (可选)
                </div>
                <div class="grid grid-cols-12 gap-3">
                  <div class="col-span-12 md:col-span-6">
                    <n-form-item label="用户名" size="small">
                      <n-input v-model:value="config.proxy_username" placeholder="无" clearable />
                    </n-form-item>
                  </div>
                  <div class="col-span-12 md:col-span-6">
                    <n-form-item label="密码" size="small">
                      <n-input v-model:value="config.proxy_password" type="password" show-password-on="click" placeholder="无" clearable />
                    </n-form-item>
                  </div>
                </div>
              </div>
            </div>

            <!-- 检测区域 -->
            <div class="p-4 rounded-lg border border-slate-200 dark:border-slate-700 bg-slate-50 dark:bg-slate-800/30">
              <div class="flex items-center justify-between mb-3">
                <div class="flex flex-col">
                  <div class="text-sm font-medium">
                    自动检测本地代理
                  </div>
                  <div class="text-xs text-gray-500">
                    自动扫描常用端口和进程
                  </div>
                </div>
                <n-button secondary size="small" :loading="proxyDetecting" @click="detectProxy">
                  <template #icon>
                    <div class="i-carbon-radar" />
                  </template>
                  开始检测
                </n-button>
              </div>

              <n-form-item label="额外扫描端口 (可选)" label-placement="left" size="small" :show-feedback="false">
                <n-input v-model:value="extraDetectPortsText" placeholder="8888, 8081" class="max-w-[300px]" />
              </n-form-item>

              <!-- 检测结果展示 -->
              <n-collapse-transition :show="detectedProxies.length > 0">
                <div class="mt-3 flex flex-wrap gap-2">
                  <div
                    v-for="(p, idx) in detectedProxies" :key="idx"
                    class="px-3 py-1.5 rounded-full text-xs font-mono cursor-pointer border transition-colors flex items-center gap-2"
                    :class="config.proxy_port === p.port ? 'bg-blue-100 text-blue-700 border-blue-200 dark:bg-blue-900/40 dark:text-blue-300 dark:border-blue-700' : 'bg-white text-gray-600 border-gray-200 hover:bg-gray-50 dark:bg-slate-800 dark:text-gray-300 dark:border-slate-700'"
                    @click="applyProxy(p)"
                  >
                    <span>{{ p.host }}:{{ p.port }}</span>
                    <span class="opacity-70">{{ p.proxy_type.toUpperCase() }}</span>
                    <span v-if="p.response_time_ms" class="px-1 rounded bg-black/5 dark:bg-white/10">{{ p.response_time_ms }}ms</span>
                  </div>
                </div>
              </n-collapse-transition>
            </div>
          </n-space>
        </n-tab-pane>

        <!-- Tab 2: 测速与诊断 -->
        <n-tab-pane name="speedtest" tab="网络测速与诊断">
          <n-space vertical size="medium" class="pt-2">
            <!-- 配置项 -->
            <div class="grid grid-cols-12 gap-4">
              <div class="col-span-12 md:col-span-4">
                <n-form-item label="测试模式" size="small">
                  <n-select v-model:value="speedTestMode" :options="[{ label: '对比 (代理 vs 直连)', value: 'compare' }, { label: '仅代理', value: 'proxy' }, { label: '仅直连', value: 'direct' }]" />
                </n-form-item>
              </div>

              <div class="col-span-12 md:col-span-8">
                <n-form-item label="测试项目" size="small">
                  <n-input-group>
                    <n-input v-model:value="speedTestProjectRoot" placeholder="请选择已索引项目用于上传测速" readonly />
                    <n-button secondary @click="openProjectPicker">
                      选择
                    </n-button>
                  </n-input-group>
                </n-form-item>
              </div>

              <div class="col-span-12">
                <n-form-item label="测试查询语 (每行一条，最多5条)" size="small">
                  <n-input v-model:value="speedTestQuery" type="textarea" :rows="2" placeholder="Ping; Upload; Search Query..." />
                </n-form-item>
              </div>
            </div>

            <!-- 启动区 -->
            <div class="flex items-center gap-3">
              <n-tooltip :disabled="!speedTestDisabled">
                <template #trigger>
                  <n-button type="primary" :loading="proxyTesting" :disabled="speedTestDisabled" class="px-6" @click="runSpeedTest">
                    <template #icon>
                      <div class="i-carbon-rocket" />
                    </template>
                    开始测速
                  </n-button>
                </template>
                {{ speedTestDisabledReason }}
              </n-tooltip>

              <span v-if="proxyTesting" class="text-xs text-gray-500 animate-pulse">{{ speedTestProgress }}</span>
            </div>

            <!-- 结果区 -->
            <div v-if="speedTestResult" class="mt-2 text-sm">
              <div class="flex items-center justify-between mb-2">
                <div class="font-bold flex items-center gap-2">
                  测试结果
                  <n-tag :type="speedTestResult.success ? 'success' : 'warning'" size="small" round>
                    {{ speedTestResult.success ? 'Success' : 'Partial Fail' }}
                  </n-tag>
                </div>
                <div class="flex gap-2">
                  <n-button size="tiny" secondary @click="copySpeedTestReport">
                    复制JSON
                  </n-button>
                  <n-button size="tiny" secondary @click="downloadSpeedTestReport">
                    导出报告
                  </n-button>
                </div>
              </div>

              <!-- 主要指标卡片 -->
              <div class="grid grid-cols-1 sm:grid-cols-2 gap-3 mb-4">
                <div
                  v-for="(metric, idx) in speedTestMetricsForDisplay" :key="idx"
                  class="p-3 rounded border border-slate-200 dark:border-slate-700 bg-white dark:bg-slate-800 relative group"
                >
                  <div class="flex justify-between items-start mb-2">
                    <span class="font-medium text-gray-700 dark:text-gray-200">{{ metric.name }}</span>
                    <div class="opacity-0 group-hover:opacity-100 transition-opacity absolute top-2 right-2">
                      <n-button text size="tiny" @click="copyMetricResult(metric)">
                        <div class="i-carbon-copy" />
                      </n-button>
                    </div>
                  </div>

                  <div class="flex items-end justify-between font-mono text-xs">
                    <!-- 代理耗时 -->
                    <div v-if="speedTestResult.mode !== 'direct'">
                      <div class="text-gray-400 scale-[0.8] origin-left">
                        PROXY
                      </div>
                      <div :class="metric.proxy_time_ms !== null ? 'text-blue-600 dark:text-blue-400 font-bold text-base' : 'text-gray-300'">
                        {{ metric.proxy_time_ms !== null ? `${metric.proxy_time_ms}ms` : '-' }}
                      </div>
                    </div>

                    <!-- 差异可视 -->
                    <div v-if="speedTestResult.mode === 'compare'" class="flex-1 text-center px-2 pb-1">
                      <div class="text-[10px] font-bold" :style="{ color: getDiffColor(metric.proxy_time_ms, metric.direct_time_ms) }">
                        {{ calcDiff(metric.proxy_time_ms, metric.direct_time_ms) }}
                      </div>
                    </div>

                    <!-- 直连耗时 -->
                    <div v-if="speedTestResult.mode !== 'proxy'" class="text-right">
                      <div class="text-gray-400 scale-[0.8] origin-right">
                        DIRECT
                      </div>
                      <div :class="metric.direct_time_ms !== null ? 'text-orange-600 dark:text-orange-400 font-bold text-base' : 'text-gray-300'">
                        {{ metric.direct_time_ms !== null ? `${metric.direct_time_ms}ms` : '-' }}
                      </div>
                    </div>
                  </div>

                  <div v-if="metric.error" class="mt-2 text-[10px] text-red-500 leading-tight border-t border-red-100 dark:border-red-900/30 pt-1">
                    {{ metric.error }}
                  </div>
                </div>
              </div>

              <!-- 多查询详情折叠 -->
              <div v-if="multiQuerySearchSummary" class="border border-slate-200 dark:border-slate-700 rounded-lg overflow-hidden">
                <div
                  class="bg-gray-50 dark:bg-slate-800/50 px-3 py-2 flex justify-between items-center cursor-pointer hover:bg-gray-100 dark:hover:bg-slate-800 transition-colors"
                  @click="multiQueryDetailsExpanded = !multiQueryDetailsExpanded"
                >
                  <span class="text-xs font-medium">查看 {{ multiQuerySearchDetails.length }} 条查询明细</span>
                  <div class="i-carbon-chevron-down transition-transform" :class="{ 'rotate-180': multiQueryDetailsExpanded }" />
                </div>
                <n-collapse-transition :show="multiQueryDetailsExpanded">
                  <div class="p-2 space-y-1 bg-white dark:bg-slate-900">
                    <div v-for="(d, i) in multiQuerySearchDetails" :key="i" class="flex items-center justify-between text-xs p-1.5 hover:bg-gray-50 dark:hover:bg-slate-800 rounded">
                      <span class="truncate max-w-[150px] text-gray-500" :title="d.query">{{ d.query }}</span>
                      <div class="flex gap-3 font-mono">
                        <span v-if="speedTestResult.mode !== 'direct'" class="text-blue-600">{{ d.proxy_time_ms ?? '-' }}ms</span>
                        <span v-if="speedTestResult.mode !== 'proxy'" class="text-orange-600">{{ d.direct_time_ms ?? '-' }}ms</span>
                      </div>
                    </div>
                  </div>
                </n-collapse-transition>
              </div>

              <!-- 建议 -->
              <div class="mt-3 text-xs text-gray-600 dark:text-gray-300 p-2 bg-yellow-50 dark:bg-yellow-900/10 rounded border border-yellow-100 dark:border-yellow-900/20">
                💡 {{ speedTestResult.recommendation }}
              </div>
            </div>
          </n-space>
        </n-tab-pane>
      </n-tabs>
    </div>

    <!-- 子弹窗：多代理选择 -->
    <n-modal v-model:show="proxyPickerVisible" preset="card" title="选择代理" style="width: 400px" size="small">
      <n-radio-group v-model:value="selectedProxyIndex">
        <n-space vertical>
          <n-radio v-for="(p, idx) in detectedProxies" :key="idx" :value="idx">
            {{ p.host }}:{{ p.port }} ({{ p.proxy_type }}) - {{ p.response_time_ms }}ms
          </n-radio>
        </n-space>
      </n-radio-group>
      <template #footer>
        <div class="flex justify-end gap-2">
          <n-button size="small" secondary @click="proxyPickerVisible = false">
            取消
          </n-button>
          <n-button size="small" type="primary" @click="confirmProxySelection">
            确认
          </n-button>
        </div>
      </template>
    </n-modal>

    <!-- 子弹窗：项目选择器 -->
    <n-modal v-model:show="projectPickerVisible" preset="card" title="选择测试项目" style="width: 500px" size="small">
      <div class="h-[300px] overflow-y-auto pr-2">
        <n-radio-group v-model:value="projectPickerSelected">
          <n-space vertical>
            <n-radio v-for="p in indexedProjects" :key="p.project_root" :value="p.project_root">
              <div class="text-xs">
                <div class="font-medium">
                  {{ getProjectName(p.project_root) }}
                </div>
                <div class="text-gray-400">
                  {{ p.total_files }} files · {{ formatIndexTime(p.last_success_time) }}
                </div>
              </div>
            </n-radio>
          </n-space>
        </n-radio-group>
      </div>
      <template #action>
        <div class="flex justify-between items-center w-full">
          <n-button size="small" secondary @click="addProjectVisible = true">
            添加新项目
          </n-button>
          <div class="flex gap-2">
            <n-button size="small" secondary @click="projectPickerVisible = false">
              取消
            </n-button>
            <n-button size="small" type="primary" @click="confirmProjectSelectionAndRun">
              确定
            </n-button>
          </div>
        </div>
      </template>
    </n-modal>

    <!-- 子弹窗：添加项目 -->
    <n-modal v-model:show="addProjectVisible" preset="card" title="添加新项目" style="width: 400px" size="small">
      <n-space vertical>
        <n-input v-model:value="addProjectPath" placeholder="输入绝对路径..." />
        <n-button block type="primary" :loading="addProjectIndexing" @click="addProjectAndIndexAndRun">
          索引并添加到测试
        </n-button>
      </n-space>
    </n-modal>
  </n-modal>
</template>

<style scoped>
/* 深度选择器覆盖 Naive UI 样式以匹配 UI 要求 */
:deep(.n-tabs-nav) {
  padding-left: 4px;
}
:deep(.n-card__content) {
  padding: 0 !important;
}
</style>
