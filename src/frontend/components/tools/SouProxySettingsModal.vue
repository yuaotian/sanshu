<!-- eslint-disable vue/no-mutating-props -->
<!-- eslint-disable style/max-statements-per-line -->
<script setup lang="ts">
/**
 * 代理设置独立弹窗组件
 * 包含：代理配置、自动检测、测速、测速报告等功能
 */
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { useDialog, useMessage } from 'naive-ui';
import { computed, onUnmounted, ref } from 'vue';

import AppModal from '../common/AppModal.vue';

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

// 搜索结果预览片段
interface SearchResultSnippet {
  file_path: string
  snippet: string
  line_number: number | null
}

// 搜索结果预览
interface SearchResultPreview {
  total_matches: number
  snippets: SearchResultSnippet[]
  response_length: number
}

interface SpeedTestMetric {
  name: string
  metric_type: string
  proxy_time_ms: number | null
  direct_time_ms: number | null
  success: boolean
  error: string | null
  search_result_preview?: SearchResultPreview | null
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

// 测速阶段状态
type SpeedTestStageStatus = 'Pending' | 'Running' | 'Completed' | 'Failed'

// 测速进度事件数据
interface SpeedTestProgress {
  stage: number
  stage_name: string
  percentage: number
  status: SpeedTestStageStatus
  detail: string | null
  sub_step: string | null
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
const speedTestProgressData = ref<SpeedTestProgress | null>(null)
let unlistenSpeedTestProgress: (() => void) | null = null
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

// 组件卸载时清理监听器
onUnmounted(() => {
  if (unlistenSpeedTestProgress) {
    unlistenSpeedTestProgress()
    unlistenSpeedTestProgress = null
  }
})

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

const currentProjectInfo = computed(() => {
  if (!speedTestProjectRoot.value)
    return null
  return indexedProjects.value.find(p => p.project_root === speedTestProjectRoot.value)
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
  console.log('[SouProxy] 🔄 开始加载已索引项目列表...')

  try {
    const statusResult = await invoke<{ projects: Record<string, ProjectIndexStatusLite> }>('get_all_acemcp_index_status')

    // 详细日志：打印原始返回数据
    console.log('[SouProxy] 📦 后端返回原始数据:', statusResult)
    console.log('[SouProxy] 📊 项目总数（原始）:', Object.keys(statusResult.projects || {}).length)

    const allProjects = Object.values(statusResult.projects || {})
    console.log('[SouProxy] 📋 所有项目列表:', allProjects.map(p => ({
      root: p.project_root,
      status: p.status,
      total_files: p.total_files,
      last_success_time: p.last_success_time,
    })))

    // 过滤条件：保留已索引文件数 > 0 的项目
    // 注意：如果项目正在索引中（status: indexing），可能 total_files 还未更新
    const list = allProjects.filter((p) => {
      const hasFiles = (p.total_files || 0) > 0
      console.log(`[SouProxy] 📁 项目 ${getProjectName(p.project_root)}: total_files=${p.total_files}, status=${p.status}, 通过过滤=${hasFiles}`)
      return hasFiles
    })

    console.log('[SouProxy] ✅ 过滤后项目数:', list.length)
    console.log('[SouProxy] 📝 过滤后项目列表:', list.map(p => getProjectName(p.project_root)))

    indexedProjects.value = list
  }
  catch (e) {
    console.error('[SouProxy] ❌ 加载已索引项目失败:', e)
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
  speedTestProgressData.value = null
  multiQuerySearchDetails.value = []
  multiQueryDetailsExpanded.value = false
  
  // 注册进度事件监听器
  unlistenSpeedTestProgress = await listen<SpeedTestProgress>('speed_test_progress', (event) => {
    const progress = event.payload
    speedTestProgressData.value = progress
    
    // 构建进度文本
    const statusIcon = progress.status === 'Running' ? '⏳' 
      : progress.status === 'Completed' ? '✅' 
      : progress.status === 'Failed' ? '❌' 
      : '⏸️'
    
    const subStepText = progress.sub_step ? ` - ${progress.sub_step}` : ''
    const detailText = progress.detail ? ` (${progress.detail})` : ''
    
    speedTestProgress.value = `${statusIcon} ${progress.stage_name}${subStepText}${detailText} [${progress.percentage}%]`
  })

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
    // 清理进度事件监听器
    if (unlistenSpeedTestProgress) {
      unlistenSpeedTestProgress()
      unlistenSpeedTestProgress = null
    }
    proxyTesting.value = false
    speedTestProgress.value = ''
    speedTestProgressData.value = null
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
  const normalizedPath = normalizePathForDisplay(projectRoot)
  const parts = (normalizedPath || '').replace(/\\/g, '/').split('/').filter(Boolean)
  return parts.length > 0 ? parts[parts.length - 1] : normalizedPath
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

function getDiffTagType(proxyMs: number | null, directMs: number | null): 'default' | 'success' | 'error' {
  if (proxyMs === null || directMs === null)
    return 'default'
  if (directMs === 0)
    return 'default'
  if (proxyMs < directMs)
    return 'success'
  if (proxyMs > directMs)
    return 'error'
  return 'default'
}

function proxyResponseTagType(ms: number | null | undefined): 'success' | 'warning' | 'default' {
  if (ms == null || ms < 0)
    return 'default'
  if (ms < 100)
    return 'success'
  if (ms < 300)
    return 'warning'
  return 'default'
}

// 获取进度步骤状态（用于步骤指示器）
function getStepStatus(stepName: string): 'pending' | 'current' | 'completed' {
  const currentStage = speedTestProgressData.value?.stage ?? -1
  const stageMap: Record<string, number> = {
    '初始化': 0,
    'Ping': 1,
    '搜索': 2,
    '单文件': 3,
    '项目': 4,
    '报告': 5,
  }
  const stepStage = stageMap[stepName] ?? -1
  
  if (stepStage < currentStage) return 'completed'
  if (stepStage === currentStage) return 'current'
  return 'pending'
}

// 格式化字节数为可读字符串
function formatBytes(bytes: number): string {
  if (bytes === 0)
    return '0B'
  const k = 1024
  const sizes = ['B', 'KB', 'MB', 'GB']
  const i = Math.floor(Math.log(bytes) / Math.log(k))
  const size = bytes / Math.pow(k, i)
  return `${size.toFixed(i > 0 ? 1 : 0)}${sizes[i]}`
}

// 规范化路径显示，移除Windows长路径前缀
function normalizePathForDisplay(path: string): string {
  if (!path)
    return path
  // 移除 Windows 长路径前缀 \\?\ 或 //?/
  return path.replace(/^\\\\\?\\|^\/\?\//, '')
}

function formatRelativeTime(timeStr: string | null): string {
  if (!timeStr)
    return '从未'
  try {
    const date = new Date(timeStr)
    const now = new Date()
    const diffMs = now.getTime() - date.getTime()
    const diffSec = Math.floor(diffMs / 1000)
    const diffMin = Math.floor(diffSec / 60)
    const diffHour = Math.floor(diffMin / 60)
    const diffDay = Math.floor(diffHour / 24)

    if (diffSec < 60)
      return '刚刚'
    if (diffMin < 60)
      return `${diffMin} 分钟前`
    if (diffHour < 24)
      return `${diffHour} 小时前`
    if (diffDay < 30)
      return `${diffDay} 天前`
    return date.toLocaleDateString()
  }
  catch {
    return '未知'
  }
}
</script>

<template>
  <AppModal
    v-model:show="showModal"
    title="代理设置与网络诊断"
    width="760px"
    body-overflow="hidden"
    role="dialog"
    aria-modal="true"
  >
    <div class="max-h-[calc(85vh-120px)] overflow-y-auto">
      <n-card size="small" class="mb-5">
        <div class="flex items-center justify-between gap-4">
          <div class="flex items-center gap-4 min-w-0">
            <div class="i-fa6-solid-network-wired text-2xl text-primary flex-shrink-0" />
            <div class="min-w-0">
              <div class="font-medium text-base mb-1 text-on-surface">
                启用代理服务
              </div>
              <div class="text-xs text-on-surface-muted flex flex-col gap-1">
                <span>启用后，所有 ACE API 请求将通过此代理。</span>
                <n-tag v-if="!config.proxy_enabled" size="small" type="warning">
                  当前直接连接
                </n-tag>
                <n-tag v-else size="small" type="success">
                  代理已启用 ({{ config.proxy_type.toUpperCase() }}://{{ config.proxy_host }}:{{ config.proxy_port }})
                </n-tag>
              </div>
            </div>
          </div>
          <n-switch v-model:value="config.proxy_enabled" size="small" class="flex-shrink-0">
            <template #checked>
              开启
            </template>
            <template #unchecked>
              关闭
            </template>
          </n-switch>
        </div>
      </n-card>

      <n-tabs type="segment" animated>
        <!-- Tab 1: 代理配置 -->
        <n-tab-pane name="config" tab="配置参数">
          <n-space vertical size="medium" class="pt-2">
            <!-- 代理表单 -->
            <div class="grid grid-cols-1 md:grid-cols-2 gap-4">
              <!-- 基础信息 -->
              <div class="md:col-span-2">
                <div class="text-xs font-semibold text-on-surface-muted uppercase tracking-wider mb-2">
                  基础连接
                </div>
                <div class="grid grid-cols-12 gap-3">
                  <div class="col-span-12 md:col-span-5">
                    <div>
                      <div class="text-xs text-on-surface-secondary mb-1">
                        Host (地址)
                      </div>
                      <n-input v-model:value="config.proxy_host" size="small" placeholder="127.0.0.1" clearable />
                    </div>
                  </div>
                  <div class="col-span-12 md:col-span-3">
                    <div>
                      <div class="text-xs text-on-surface-secondary mb-1">
                        Port (端口)
                      </div>
                      <n-input-number v-model:value="config.proxy_port" size="small" :min="1" :max="65535" class="w-full" :show-button="false" />
                    </div>
                  </div>
                  <div class="col-span-12 md:col-span-4">
                    <div>
                      <div class="text-xs text-on-surface-secondary mb-1">
                        Type (类型)
                      </div>
                      <n-select v-model:value="config.proxy_type" size="small" :options="[{ label: 'HTTP', value: 'http' }, { label: 'HTTPS', value: 'https' }, { label: 'SOCKS5', value: 'socks5' }]" />
                    </div>
                  </div>
                </div>
              </div>

              <!-- 认证信息 -->
              <div class="md:col-span-2">
                <div class="text-xs font-semibold text-on-surface-muted uppercase tracking-wider mb-2 mt-2">
                  身份认证 (可选)
                </div>
                <div class="grid grid-cols-12 gap-3">
                  <div class="col-span-12 md:col-span-6">
                    <div>
                      <div class="text-xs text-on-surface-secondary mb-1">
                        用户名
                      </div>
                      <n-input v-model:value="config.proxy_username" size="small" placeholder="无" clearable />
                    </div>
                  </div>
                  <div class="col-span-12 md:col-span-6">
                    <div>
                      <div class="text-xs text-on-surface-secondary mb-1">
                        密码
                      </div>
                      <n-input v-model:value="config.proxy_password" size="small" type="password" show-password-on="click" placeholder="无" clearable />
                    </div>
                  </div>
                </div>
              </div>
            </div>

            <n-card size="small">
              <template #header>
                <div class="flex items-center justify-between gap-3 w-full">
                  <div class="min-w-0">
                    <div class="text-sm font-medium text-on-surface">
                      自动检测本地代理
                    </div>
                    <div class="text-xs text-on-surface-muted">
                      自动扫描常用端口和进程
                    </div>
                  </div>
                  <n-button secondary size="small" :loading="proxyDetecting" class="flex-shrink-0" @click="detectProxy">
                    <template #icon>
                      <div class="i-fa6-solid-satellite-dish" />
                    </template>
                    开始检测
                  </n-button>
                </div>
              </template>
              <div>
                <div class="text-xs text-on-surface-secondary mb-1">
                  额外扫描端口 (可选)
                </div>
                <n-input v-model:value="extraDetectPortsText" size="small" placeholder="8888, 8081" class="max-w-[300px]" />
              </div>
              <n-collapse-transition :show="detectedProxies.length > 0">
                <n-space class="mt-3" size="small" wrap>
                  <n-tag
                    v-for="(p, idx) in detectedProxies"
                    :key="idx"
                    size="small"
                    :type="config.proxy_port === p.port ? 'primary' : 'default'"
                    class="font-mono cursor-pointer"
                    @click="applyProxy(p)"
                  >
                    {{ p.host }}:{{ p.port }}
                    <span class="opacity-70 ml-1">{{ p.proxy_type.toUpperCase() }}</span>
                    <span v-if="p.response_time_ms" class="ml-1">{{ p.response_time_ms }}ms</span>
                  </n-tag>
                </n-space>
              </n-collapse-transition>
            </n-card>
          </n-space>
        </n-tab-pane>

        <!-- Tab 2: 测速与诊断 - 左右分栏布局 -->
        <n-tab-pane name="speedtest" tab="网络测速与诊断">
          <div class="grid grid-cols-12 gap-5 pt-2">
            <!-- 左侧：测试控制区 (40%) -->
            <div class="col-span-12 lg:col-span-5 space-y-5">
              <!-- 测试模式选择 -->
              <div class="space-y-2">
                <div class="text-xs font-semibold text-on-surface-muted dropdown-label flex items-center gap-1">
                  <div class="i-fa6-solid-gauge-high" />
                  测试模式
                </div>
                <n-select
                  v-model:value="speedTestMode"
                  size="small"
                  :options="[
                    { label: '🔥 对比测试 (代理 vs 直连)', value: 'compare' },
                    { label: '🛡️ 仅代理模式', value: 'proxy' },
                    { label: '🌐 仅直连模式', value: 'direct' },
                  ]"
                />
              </div>

              <!-- 测试项目选择 (卡片式) -->
              <div class="space-y-2">
                <div class="text-xs font-semibold text-on-surface-muted dropdown-label flex items-center justify-between">
                  <div class="flex items-center gap-1">
                    <div class="i-fa6-solid-folder-tree" />
                    测试目标项目
                  </div>
                  <n-button v-if="currentProjectInfo" text size="tiny" type="primary" @click="openProjectPicker">
                    切换
                  </n-button>
                </div>

                <n-card
                  v-if="currentProjectInfo"
                  size="small"
                  hoverable
                  class="cursor-pointer"
                  @click="openProjectPicker"
                >
                  <div class="flex items-start gap-3">
                    <div class="i-fa6-solid-code text-xl text-primary flex-shrink-0 mt-0.5" />
                    <div class="flex-1 min-w-0">
                      <div class="font-medium text-base text-on-surface truncate">
                        {{ getProjectName(currentProjectInfo.project_root) }}
                      </div>
                      <div class="text-xs text-on-surface-muted truncate font-mono mt-0.5" :title="currentProjectInfo.project_root">
                        {{ normalizePathForDisplay(currentProjectInfo.project_root) }}
                      </div>
                      <div class="flex items-center gap-3 mt-2 text-xs text-on-surface-muted flex-wrap">
                        <n-tag size="tiny" :bordered="false">
                          <span class="inline-flex items-center gap-1">
                            <div class="i-fa6-solid-file-lines text-[10px]" />
                            {{ currentProjectInfo.total_files }} 文件
                          </span>
                        </n-tag>
                        <span v-if="currentProjectInfo.last_success_time" class="inline-flex items-center gap-1">
                          <div class="i-fa6-regular-clock text-[10px]" />
                          {{ formatRelativeTime(currentProjectInfo.last_success_time) }}
                        </span>
                      </div>
                    </div>
                  </div>
                </n-card>

                <n-card
                  v-else
                  size="small"
                  hoverable
                  class="cursor-pointer"
                  @click="openProjectPicker"
                >
                  <n-empty size="small" description="点击选择测试项目">
                    <template #icon>
                      <div class="i-fa6-solid-folder-plus text-3xl text-on-surface-muted" />
                    </template>
                  </n-empty>
                </n-card>
              </div>

              <!-- 查询语句 -->
              <div class="space-y-2">
                <div class="flex items-center justify-between text-xs font-semibold text-on-surface-muted dropdown-label">
                  <div class="flex items-center gap-1">
                    <div class="i-fa6-solid-magnifying-glass" />
                    测试查询语句
                  </div>
                  <span class="font-normal opacity-70">最多5条</span>
                </div>
                <n-input
                  v-model:value="speedTestQuery"
                  size="small"
                  type="textarea"
                  :rows="3"
                  placeholder="输入语义查询，如：'查找数据库连接配置'..."
                  class="text-sm"
                />
              </div>

              <!-- 开始测速按钮 -->
              <n-tooltip :disabled="!speedTestDisabled">
                <template #trigger>
                  <n-button
                    type="primary"
                    block
                    size="small"
                    :loading="proxyTesting"
                    :disabled="speedTestDisabled"
                    class="h-12 text-base font-medium"
                    @click="runSpeedTest"
                  >
                    <template #icon>
                      <div class="i-fa6-solid-jet-fighter" />
                    </template>
                    {{ proxyTesting ? '全速诊断中...' : '开始网络诊断' }}
                  </n-button>
                </template>
                {{ speedTestDisabledReason }}
              </n-tooltip>

              <div v-if="proxyTesting" class="space-y-3">
                <!-- 进度头部 -->
                <div class="flex justify-between items-center text-xs">
                  <span class="text-on-surface-muted font-medium">诊断进度</span>
                  <span class="font-mono text-primary">
                    {{ speedTestProgressData?.percentage ?? 0 }}%
                  </span>
                </div>
                
                <!-- 进度条 -->
                <n-progress
                  type="line"
                  :percentage="speedTestProgressData?.percentage ?? 5"
                  :show-indicator="false"
                  :processing="speedTestProgressData?.status === 'Running'"
                  :status="speedTestProgressData?.status === 'Failed' ? 'error' : 'success'"
                  class="h-2"
                />
                
                <n-card size="small" embedded>
                  <div class="space-y-1">
                    <div class="flex items-center gap-2 text-sm">
                      <span
                        v-if="speedTestProgressData?.status === 'Running'"
                        class="i-fa6-solid-spinner animate-spin text-primary"
                      />
                      <span
                        v-else-if="speedTestProgressData?.status === 'Completed'"
                        class="i-fa6-solid-circle-check text-success"
                      />
                      <span
                        v-else-if="speedTestProgressData?.status === 'Failed'"
                        class="i-fa6-solid-circle-xmark text-error"
                      />
                      <span v-else class="i-fa6-regular-clock text-on-surface-muted" />

                      <span class="font-medium text-on-surface">
                        {{ speedTestProgressData?.stage_name ?? '初始化' }}
                      </span>
                      <span v-if="speedTestProgressData?.sub_step" class="text-on-surface-muted">
                        - {{ speedTestProgressData.sub_step }}
                      </span>
                    </div>

                    <div v-if="speedTestProgressData?.detail" class="text-xs text-on-surface-muted pl-6">
                      {{ speedTestProgressData.detail }}
                    </div>
                  </div>
                </n-card>
                
                <!-- 进度步骤指示器 -->
                <div class="flex justify-between px-1">
                  <div v-for="step in ['初始化', 'Ping', '搜索', '单文件', '项目', '报告']" :key="step" class="flex flex-col items-center gap-1">
                    <div
                      class="i-fa6-solid-circle text-[8px] transition-all"
                      :class="getStepStatus(step) === 'completed'
                        ? 'text-success scale-125'
                        : getStepStatus(step) === 'current'
                          ? 'text-primary animate-pulse scale-110'
                          : 'text-on-surface-muted opacity-40'"
                    />
                    <span class="text-[10px] text-on-surface-muted">{{ step }}</span>
                  </div>
                </div>
              </div>
            </div>

            <!-- 右侧：测试结果区 (60%) -->
            <div class="col-span-12 lg:col-span-7 h-full flex flex-col">
              <n-card
                v-if="!speedTestResult && !proxyTesting"
                size="small"
                class="flex-1 flex flex-col"
              >
                <div class="flex flex-col items-center justify-center py-8 px-4 flex-1">
                  <n-empty>
                    <template #icon>
                      <div class="i-fa6-solid-chart-simple text-6xl text-on-surface-muted" />
                    </template>
                    <template #default>
                      <div class="text-base font-medium text-on-surface-secondary mb-2">
                        准备就绪
                      </div>
                      <div class="text-xs text-on-surface-muted max-w-xs text-center">
                        请在左侧配置测试参数，点击「开始网络诊断」获取详细的延迟与连通性分析报告。
                      </div>
                    </template>
                  </n-empty>
                </div>
              </n-card>

              <n-card v-else-if="proxyTesting && !speedTestResult" size="small" class="flex-1">
                <div class="space-y-4">
                  <div class="flex items-center gap-4">
                    <n-skeleton circle width="48px" height="48px" />
                    <div class="flex-1 space-y-2">
                      <n-skeleton height="20px" width="60%" />
                      <n-skeleton height="14px" width="40%" />
                    </div>
                  </div>
                  <div class="grid grid-cols-2 gap-4">
                    <n-skeleton height="120px" :sharp="false" />
                    <n-skeleton height="120px" :sharp="false" />
                  </div>
                  <n-skeleton height="200px" :sharp="false" class="mt-4" />
                </div>
              </n-card>

              <n-card v-if="speedTestResult" size="small" class="speed-test-tabs-host flex-1 flex flex-col overflow-hidden">
                <template #header>
                  <div class="flex items-center justify-between gap-3 w-full">
                    <div class="flex items-center gap-4 min-w-0">
                      <div
                        class="text-2xl flex-shrink-0"
                        :class="speedTestResult.success ? 'i-fa6-solid-check text-success' : 'i-fa6-solid-triangle-exclamation text-warning'"
                      />
                      <div class="min-w-0">
                        <div class="font-bold text-lg leading-none mb-1 text-on-surface">
                          {{ speedTestResult.success ? '测试通过' : '发现问题' }}
                        </div>
                        <div class="text-xs text-on-surface-muted font-mono">
                          TIME: {{ formatSpeedTestTime(speedTestResult.timestamp) }}
                        </div>
                      </div>
                    </div>
                    <n-space class="flex-shrink-0" size="small">
                      <n-button size="small" secondary @click="copySpeedTestReport">
                        复制报告
                      </n-button>
                      <n-button size="small" secondary @click="downloadSpeedTestReport">
                        <template #icon>
                          <div class="i-fa6-solid-download" />
                        </template>
                      </n-button>
                    </n-space>
                  </div>
                </template>

                <n-tabs type="line" animated class="flex-1 flex flex-col results-tabs" pane-class="flex-1 p-4 overflow-y-auto max-h-[500px]">
                  <!-- Tab 1: 核心指标 -->
                  <n-tab-pane name="overview" tab="📊 核心指标">
                    <div class="space-y-4">
                      <n-alert
                        v-if="speedTestResult.recommendation"
                        type="info"
                        title="智能诊断建议"
                        :show-icon="true"
                      >
                        <template #icon>
                          <div class="i-fa6-solid-wand-magic-sparkles text-primary" />
                        </template>
                        {{ speedTestResult.recommendation }}
                      </n-alert>

                      <div class="grid grid-cols-2 gap-4">
                        <n-card
                          v-for="(metric, idx) in speedTestMetricsForDisplay"
                          :key="idx"
                          size="small"
                          hoverable
                        >
                          <div class="flex justify-between items-start mb-4">
                            <span class="font-semibold text-sm text-on-surface">{{ metric.name }}</span>
                            <div v-if="metric.success" class="i-fa6-solid-circle-check text-success" />
                            <div v-else class="i-fa6-solid-circle-xmark text-error" />
                          </div>

                          <div class="flex items-end justify-between font-mono text-sm">
                            <div v-if="speedTestResult.mode !== 'direct'" class="flex-1">
                              <div class="text-xs text-on-surface-muted mb-1">
                                Proxy
                              </div>
                              <div class="text-xl font-bold" :class="metric.proxy_time_ms ? 'text-info' : 'text-on-surface-muted'">
                                {{ metric.proxy_time_ms ?? '-' }}<span class="text-xs font-normal text-on-surface-muted">ms</span>
                              </div>
                            </div>

                            <div v-if="speedTestResult.mode === 'compare'" class="px-2 pb-1">
                              <n-tag size="small" :type="getDiffTagType(metric.proxy_time_ms, metric.direct_time_ms)">
                                {{ calcDiff(metric.proxy_time_ms, metric.direct_time_ms) }}
                              </n-tag>
                            </div>

                            <div v-if="speedTestResult.mode !== 'proxy'" class="flex-1 text-right">
                              <div class="text-xs text-on-surface-muted mb-1">
                                Direct
                              </div>
                              <div class="text-xl font-bold" :class="metric.direct_time_ms ? 'text-primary' : 'text-on-surface-muted'">
                                {{ metric.direct_time_ms ?? '-' }}<span class="text-xs font-normal text-on-surface-muted">ms</span>
                              </div>
                            </div>
                          </div>

                          <n-alert v-if="metric.error" type="error" class="mt-3">
                            {{ metric.error }}
                          </n-alert>
                        </n-card>
                      </div>

                      <div v-if="multiQuerySearchSummary" class="mt-4">
                        <div class="text-xs font-semibold text-on-surface-muted mb-2 uppercase tracking-wider">
                          Search Queries
                        </div>
                        <n-space vertical size="small">
                          <n-card
                            v-for="(d, i) in multiQuerySearchDetails"
                            :key="i"
                            size="small"
                            embedded
                          >
                            <div class="flex items-center justify-between gap-3">
                              <div class="flex items-center gap-2 truncate flex-1 min-w-0">
                                <div class="i-fa6-solid-terminal text-on-surface-muted text-xs flex-shrink-0" />
                                <span class="text-xs font-mono truncate" :title="d.query">{{ d.query }}</span>
                              </div>
                              <div class="flex gap-3 text-xs font-mono flex-shrink-0">
                                <span v-if="d.proxy_time_ms" class="text-info">{{ d.proxy_time_ms }}ms</span>
                                <span v-if="d.direct_time_ms" class="text-primary">{{ d.direct_time_ms }}ms</span>
                              </div>
                            </div>
                          </n-card>
                        </n-space>
                      </div>
                    </div>
                  </n-tab-pane>

                  <!-- Tab 2: 完整诊断数据 -->
                  <n-tab-pane name="raw" tab="🛠️ 诊断数据">
                    <div class="space-y-4">
                      <n-alert title="数据说明" type="info" :bordered="false" class="mb-2">
                        以下展示测试过程中的完整配置上下文与后端返回的原始指标数据结构。
                      </n-alert>

                      <div>
                        <div class="flex items-center justify-between mb-2">
                          <span class="text-xs font-bold text-on-surface-muted">REQUEST CONTEXT</span>
                          <n-tag size="tiny">
                            JSON
                          </n-tag>
                        </div>
                        <n-code
                          :code="JSON.stringify({
                            mode: speedTestMode,
                            query: speedTestQuery,
                            project: currentProjectInfo ? { root: currentProjectInfo.project_root, files: currentProjectInfo.total_files } : null,
                            timestamp: new Date().toISOString(),
                          }, null, 2)"
                          language="json"
                          class="text-xs font-mono"
                          style="max-height: 200px; overflow: auto;"
                        />
                      </div>

                      <div>
                        <div class="flex items-center justify-between mb-2">
                          <span class="text-xs font-bold text-on-surface-muted">RESPONSE METRICS (RAW)</span>
                          <n-button size="tiny" text type="primary" @click="copySpeedTestReport">
                            复制完整JSON
                          </n-button>
                        </div>
                        <n-code
                          :code="JSON.stringify(speedTestResult, null, 2)"
                          language="json"
                          class="text-xs font-mono"
                        />
                      </div>
                    </div>
                  </n-tab-pane>

                  <!-- Tab 3: 搜索数据 -->
                  <n-tab-pane name="search-data" tab="🔍 搜索数据">
                    <div class="space-y-4">
                      <!-- 搜索结果预览卡片 -->
                      <n-card
                        v-for="(metric, idx) in speedTestResult.metrics.filter(m => m.metric_type === 'search' && m.search_result_preview)"
                        :key="idx"
                        size="small"
                      >
                        <template #header>
                          <div class="flex items-center justify-between gap-3 w-full">
                            <div class="flex items-center gap-2 min-w-0">
                              <div class="i-fa6-solid-magnifying-glass text-info flex-shrink-0" />
                              <span class="font-medium text-sm text-on-surface truncate">{{ metric.name }}</span>
                            </div>
                            <div class="flex items-center gap-3 text-xs font-mono flex-shrink-0">
                              <span class="text-on-surface-muted">匹配: {{ metric.search_result_preview?.total_matches || 0 }}</span>
                              <span class="text-on-surface-muted">响应: {{ formatBytes(metric.search_result_preview?.response_length || 0) }}</span>
                            </div>
                          </div>
                        </template>

                        <template v-if="metric.search_result_preview?.snippets?.length">
                          <div
                            v-for="(snippet, i) in metric.search_result_preview.snippets"
                            :key="i"
                            class="py-3"
                          >
                            <n-divider v-if="i > 0" class="!my-3" />
                            <div class="flex items-center gap-2 mb-2 flex-wrap">
                              <div class="i-fa6-solid-file-code text-xs text-on-surface-muted flex-shrink-0" />
                              <span class="text-xs font-mono text-info truncate min-w-0" :title="snippet.file_path">
                                {{ snippet.file_path }}
                              </span>
                              <n-tag v-if="snippet.line_number" size="tiny" :bordered="false">
                                L{{ snippet.line_number }}
                              </n-tag>
                            </div>
                            <n-code
                              :code="snippet.snippet"
                              language="text"
                              class="text-xs font-mono"
                            />
                          </div>
                        </template>
                        <n-empty
                          v-else
                          size="small"
                          description="未获取到搜索结果预览"
                        >
                          <template #icon>
                            <div class="i-fa6-solid-inbox text-3xl text-on-surface-muted opacity-50" />
                          </template>
                        </n-empty>
                      </n-card>

                      <n-empty v-if="!speedTestResult.metrics.some(m => m.metric_type === 'search' && m.search_result_preview)">
                        <template #icon>
                          <div class="i-fa6-solid-search text-4xl text-on-surface-muted opacity-30" />
                        </template>
                        <template #default>
                          <div class="text-sm font-medium text-on-surface-secondary mb-1">
                            暂无搜索数据
                          </div>
                          <div class="text-xs text-on-surface-muted opacity-70">
                            运行包含语义搜索的测试后，将在此处展示搜索结果预览
                          </div>
                        </template>
                      </n-empty>
                    </div>
                  </n-tab-pane>
                </n-tabs>
              </n-card>
            </div>
          </div>
        </n-tab-pane>
      </n-tabs>
    </div>

    <!-- 子弹窗：多代理选择 -->
    <AppModal
      v-model:show="proxyPickerVisible"
      width="480px"
      max-height="80vh"
    >
      <template #header>
        <div class="flex items-center gap-3 min-w-0">
          <div class="i-fa6-solid-network-wired text-primary text-xl flex-shrink-0" />
          <div class="min-w-0">
            <div class="font-semibold text-base text-on-surface">
              选择代理服务器
            </div>
            <div class="text-xs text-on-surface-muted">
              已检测到 {{ detectedProxies.length }} 个可用代理
            </div>
          </div>
        </div>
      </template>

      <n-radio-group v-model:value="selectedProxyIndex" class="max-h-[300px] overflow-y-auto pr-1">
        <n-space vertical size="small" class="w-full">
          <n-radio
            v-for="(p, idx) in detectedProxies"
            :key="idx"
            :value="idx"
            class="w-full"
          >
            <n-card
              size="small"
              embedded
              class="w-full mt-1"
            >
              <div class="flex items-center justify-between gap-3">
                <div class="min-w-0">
                  <div class="font-mono font-medium text-sm text-on-surface truncate">
                    {{ p.host }}:{{ p.port }}
                  </div>
                  <div class="text-xs text-on-surface-muted mt-0.5">
                    {{ p.proxy_type.toUpperCase() }} 代理
                  </div>
                </div>
                <n-tag size="small" :type="proxyResponseTagType(p.response_time_ms)">
                  <span class="inline-flex items-center gap-1">
                    <span class="i-fa6-solid-bolt" />
                    {{ p.response_time_ms ?? '-' }}ms
                  </span>
                </n-tag>
              </div>
            </n-card>
          </n-radio>
        </n-space>
      </n-radio-group>

      <template #footer>
        <div class="flex justify-end gap-3">
          <n-button secondary size="small" @click="proxyPickerVisible = false">
            取消
          </n-button>
          <n-button type="primary" size="small" @click="confirmProxySelection">
            <template #icon>
              <div class="i-fa6-solid-check" />
            </template>
            确认选择
          </n-button>
        </div>
      </template>
    </AppModal>

    <!-- 子弹窗：项目选择器 -->
    <AppModal
      v-model:show="projectPickerVisible"
      width="600px"
    >
      <template #header>
        <div class="flex items-center gap-3 min-w-0">
          <div class="i-fa6-solid-folder-tree text-primary text-xl flex-shrink-0" />
          <div class="min-w-0">
            <div class="font-bold text-lg leading-tight text-on-surface">
              选择测试项目
            </div>
            <div class="text-xs text-on-surface-muted mt-1">
              请选择一个已索引的代码库进行网络延迟测试
            </div>
          </div>
        </div>
      </template>

      <n-card v-if="projectPickerLoading" size="small">
        <div class="grid grid-cols-1 md:grid-cols-2 gap-4 py-2">
          <n-skeleton height="100px" :sharp="false" />
          <n-skeleton height="100px" :sharp="false" />
          <n-skeleton height="100px" :sharp="false" />
          <n-skeleton height="100px" :sharp="false" />
        </div>
      </n-card>

      <div v-else-if="indexedProjects.length === 0" class="py-6">
        <n-empty>
          <template #icon>
            <div class="i-fa6-solid-folder-open text-5xl text-on-surface-muted" />
          </template>
          <template #default>
            <div class="text-base font-medium text-on-surface-secondary">
              暂无可用项目
            </div>
            <div class="text-xs text-on-surface-muted mt-2">
              请先添加项目并建立索引
            </div>
          </template>
        </n-empty>
      </div>

      <n-radio-group
        v-else
        v-model:value="projectPickerSelected"
        class="grid grid-cols-1 md:grid-cols-2 gap-4 max-h-[450px] overflow-y-auto p-1"
      >
        <n-radio
          v-for="p in indexedProjects"
          :key="p.project_root"
          :value="p.project_root"
          class="project-picker-radio w-full"
        >
          <n-card
            size="small"
            hoverable
            embedded
            class="w-full mt-1"
            @click="projectPickerSelected = p.project_root"
          >
            <div class="flex justify-between items-start gap-2">
              <div class="flex items-center gap-2 min-w-0 flex-1">
                <div class="i-fa6-solid-code-branch text-primary flex-shrink-0" />
                <div class="font-bold text-sm truncate text-on-surface" :title="getProjectName(p.project_root)">
                  {{ getProjectName(p.project_root) }}
                </div>
              </div>
              <n-tag v-if="projectPickerSelected === p.project_root" type="primary" size="tiny">
                已选
              </n-tag>
            </div>

            <div class="text-xs text-on-surface-muted font-mono truncate mt-2" :title="p.project_root">
              {{ normalizePathForDisplay(p.project_root) }}
            </div>

            <div class="mt-3 flex items-center justify-between text-xs flex-wrap gap-2">
              <n-tag size="tiny" :bordered="false">
                <span class="inline-flex items-center gap-1">
                  <span class="i-fa6-solid-file" />
                  {{ p.total_files }}
                </span>
              </n-tag>
              <span class="text-on-surface-muted inline-flex items-center gap-1">
                <div class="i-fa6-regular-clock" />
                {{ formatRelativeTime(p.last_success_time) }}
              </span>
            </div>
          </n-card>
        </n-radio>
      </n-radio-group>

      <template #action>
        <div class="flex justify-between items-center w-full">
          <n-button secondary size="small" @click="addProjectVisible = true">
            <template #icon>
              <div class="i-fa6-solid-plus" />
            </template>
            添加新项目
          </n-button>
          <div class="flex gap-3">
            <n-button secondary size="small" @click="projectPickerVisible = false">
              取消
            </n-button>
            <n-button type="primary" size="small" :disabled="!projectPickerSelected" @click="confirmProjectSelectionAndRun">
              <template #icon>
                <div class="i-fa6-solid-play" />
              </template>
              开始测试
            </n-button>
          </div>
        </div>
      </template>
    </AppModal>

    <!-- 子弹窗：添加项目 -->
    <AppModal
      v-model:show="addProjectVisible"
      width="480px"
      max-height="80vh"
    >
      <template #header>
        <div class="flex items-center gap-3 min-w-0">
          <div class="i-fa6-solid-folder-plus text-success text-xl flex-shrink-0" />
          <div class="min-w-0">
            <div class="font-semibold text-base text-on-surface">
              添加新项目
            </div>
            <div class="text-xs text-on-surface-muted">
              输入项目根目录路径进行索引
            </div>
          </div>
        </div>
      </template>

      <div class="space-y-4">
        <div>
          <div class="text-xs text-on-surface-secondary mb-1">
            项目路径
          </div>
          <n-input
            v-model:value="addProjectPath"
            size="small"
            placeholder="例如：C:\Projects\my-app 或 /home/user/projects/my-app"
            clearable
          >
            <template #prefix>
              <div class="i-fa6-solid-folder text-on-surface-muted" />
            </template>
          </n-input>
        </div>

        <n-alert type="info" :show-icon="true" :bordered="false">
          <template #icon>
            <div class="i-fa6-solid-circle-info text-info" />
          </template>
          添加后将自动创建索引，完成后可用于测速。请确保路径为项目根目录且包含代码文件。
        </n-alert>
      </div>

      <template #footer>
        <div class="flex justify-end gap-3">
          <n-button secondary size="small" @click="addProjectVisible = false">
            取消
          </n-button>
          <n-button type="primary" size="small" :loading="addProjectIndexing" :disabled="!addProjectPath.trim()" @click="addProjectAndIndexAndRun">
            <template #icon>
              <div class="i-fa6-solid-database" />
            </template>
            {{ addProjectIndexing ? '索引中...' : '创建索引并测试' }}
          </n-button>
        </div>
      </template>
    </AppModal>
  </AppModal>
</template>

<style scoped>
/* 深度选择器覆盖 Naive UI 样式以匹配 UI 要求 */
:deep(.n-tabs-nav) {
  padding-left: 4px;
}
.speed-test-tabs-host :deep(> .n-card__content) {
  padding: 0 !important;
}

/* 优化结果区 Tabs 内容区样式 - 确保文字清晰可读 */
.results-tabs :deep(.n-tab-pane) {
  border-top: 1px solid var(--color-border);
}

/* 优化代码块在暗色模式下的可读性 */
:deep(.n-code) {
  max-height: 300px;
  overflow: auto;
}

</style>
