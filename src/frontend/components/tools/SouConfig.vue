<script setup lang="ts">
/**
 * 代码搜索工具 (Acemcp/Sou) 配置组件
 * 包含：基础配置、高级配置、日志调试、索引管理
 */
import { invoke } from '@tauri-apps/api/core'
import { useMessage } from 'naive-ui'
import { computed, onMounted, ref, watch } from 'vue'
import { useAcemcpSync } from '../../composables/useAcemcpSync'
import ConfigSection from '../common/ConfigSection.vue'
import ProjectIndexManager from '../settings/ProjectIndexManager.vue'
import ProxySettingsModal from './SouProxySettingsModal.vue'

// Props
const props = defineProps<{
  active: boolean
}>()

const message = useMessage()

// Acemcp 同步状态
const {
  autoIndexEnabled,
  fetchAutoIndexEnabled,
  setAutoIndexEnabled,
  fetchWatchingProjects,
} = useAcemcpSync()

// 配置状态
const config = ref({
  base_url: '',
  token: '',
  batch_size: 10,
  max_lines_per_blob: 800,
  text_extensions: [] as string[],
  exclude_patterns: [] as string[],
  watch_debounce_minutes: 3, // 文件监听防抖延迟（分钟），默认 3 分钟
  // 代理配置
  proxy_enabled: false,
  proxy_host: '127.0.0.1',
  proxy_port: 7890,
  proxy_type: 'http' as 'http' | 'https' | 'socks5',
  proxy_username: '',
  proxy_password: '',
  // 嵌套项目索引配置
  index_nested_projects: true, // 是否自动索引嵌套的 Git 子项目（默认启用）
  // sou 多后端配置
  sou_default_backend: 'auto' as 'auto' | 'ace' | 'fast_context' | 'both',
  sou_auto_order: ['ace', 'fast_context'] as string[],
  sou_include_backend_headers: true,
  sou_include_failed_backend_errors: true,
  // fast-context 配置
  fast_context_api_key: '',
  fast_context_tree_depth: 3,
  fast_context_max_turns: 3,
  fast_context_max_results: 10,
  fast_context_max_commands: 8,
  fast_context_timeout_ms: 30000,
  fast_context_exclude_paths: ['node_modules', '.git', 'dist', 'build', 'target'] as string[],
})

const loadingConfig = ref(false)
const showProxyModal = ref(false)
const lastSavedConnection = ref({
  base_url: '',
  token: '',
})
// 调试状态
const debugProjectRoot = ref('')
const debugQuery = ref('')
const debugLoading = ref(false)
const debugBackend = ref<'default' | 'auto' | 'ace' | 'fast_context' | 'both'>('default')
const debugUseManualInput = ref(false) // 是否使用手动输入模式
const debugProjectOptions = ref<{ label: string, value: string }[]>([]) // 项目选择选项
const debugProjectOptionsLoading = ref(false) // 加载项目列表中

// 调试结果增强类型
interface DebugSearchResult {
  success: boolean
  result?: string
  error?: string
  request_time: string
  response_time: string
  total_duration_ms: number
  result_count?: number
  project_path: string
  query: string
}

interface FastContextApiKeyDetectionResult {
  found: boolean
  source?: string
  source_label?: string
  api_key?: string
  masked_api_key?: string
  saved: boolean
  message: string
}

const debugResultData = ref<DebugSearchResult | null>(null)
const detectingFastContextKey = ref(false)
const fastContextKeyStatus = ref('')
const fastContextKeyStatusType = ref<'success' | 'warning' | 'error' | 'info'>('info')

type ExtensionGroup = {
  id: string
  label: string
  description: string
  icon: string
  extensions: string[]
}

type ExtensionPreset = {
  label: string
  description: string
  icon: string
  extensions: string[]
}

// 中文说明：扩展名预设按常见工程角色分组，既补齐 Vue/Svelte 等前端文件，也避免模板里维护散落列表。
const extensionGroups: ExtensionGroup[] = [
  {
    id: 'frontend',
    label: '前端',
    description: '组件、页面、样式',
    icon: 'i-carbon-application-web',
    extensions: [
      '.js',
      '.mjs',
      '.cjs',
      '.ts',
      '.jsx',
      '.tsx',
      '.vue',
      '.svelte',
      '.astro',
      '.html',
      '.css',
      '.scss',
      '.sass',
      '.less',
      '.postcss',
    ],
  },
  {
    id: 'backend',
    label: '后端',
    description: '服务端与系统语言',
    icon: 'i-carbon-code',
    extensions: [
      '.py',
      '.java',
      '.go',
      '.rs',
      '.cpp',
      '.c',
      '.h',
      '.hpp',
      '.cs',
      '.rb',
      '.php',
      '.kt',
      '.kts',
      '.swift',
      '.scala',
      '.lua',
    ],
  },
  {
    id: 'config',
    label: '配置/数据',
    description: '配置、Schema、查询',
    icon: 'i-carbon-data-structured',
    extensions: [
      '.json',
      '.jsonc',
      '.yaml',
      '.yml',
      '.toml',
      '.xml',
      '.sql',
      '.graphql',
      '.gql',
      '.proto',
      '.ini',
    ],
  },
  {
    id: 'docs',
    label: '文档/脚本',
    description: '说明文档与自动化脚本',
    icon: 'i-carbon-document',
    extensions: [
      '.md',
      '.mdx',
      '.txt',
      '.rst',
      '.adoc',
      '.sh',
      '.bash',
      '.zsh',
      '.fish',
      '.ps1',
      '.bat',
    ],
  },
]

const allPresetExtensions = Array.from(new Set(extensionGroups.flatMap(group => group.extensions)))
const presetExtensionSet = new Set(allPresetExtensions)
const extensionPresets: ExtensionPreset[] = [
  {
    label: '现代前端',
    description: 'Vue/Svelte/React/Astro + 样式',
    icon: 'i-carbon-application-web',
    extensions: ['.js', '.mjs', '.cjs', '.ts', '.jsx', '.tsx', '.vue', '.svelte', '.astro', '.html', '.css', '.scss', '.sass', '.less', '.postcss'],
  },
  {
    label: '全栈常用',
    description: '前端、后端、配置、文档',
    icon: 'i-carbon-assembly-cluster',
    extensions: allPresetExtensions,
  },
  {
    label: 'Rust/Tauri',
    description: 'Rust + Vue + 配置脚本',
    icon: 'i-carbon-terminal',
    extensions: ['.rs', '.toml', '.json', '.jsonc', '.ts', '.tsx', '.js', '.vue', '.html', '.css', '.scss', '.md', '.ps1', '.sh'],
  },
  {
    label: '文档配置',
    description: '文档、Schema、脚本',
    icon: 'i-carbon-settings',
    extensions: ['.md', '.mdx', '.txt', '.rst', '.adoc', '.json', '.jsonc', '.yaml', '.yml', '.toml', '.xml', '.graphql', '.gql', '.proto', '.sh', '.bash', '.ps1'],
  },
]

// 选项数据
const extOptions = ref(allPresetExtensions.map(v => ({ label: v, value: v })))
const extensionSearchQuery = ref('')
const extensionGroupActionOptions = [
  { label: '反选本组', key: 'invert' },
  { label: '仅保留本组', key: 'keep' },
  { label: '移除本组', key: 'remove' },
]

const excludeOptions = ref([
  '.venv',
  'venv',
  '.env',
  'env',
  'node_modules',
  '.next',
  '.nuxt',
  '.output',
  'out',
  '.cache',
  '.turbo',
  '.vercel',
  '.netlify',
  '.swc',
  '.vite',
  '.parcel-cache',
  '.sass-cache',
  '.eslintcache',
  '.stylelintcache',
  'coverage',
  '.nyc_output',
  'tmp',
  'temp',
  '.tmp',
  '.temp',
  '.git',
  '.svn',
  '.hg',
  '__pycache__',
  '.pytest_cache',
  '.mypy_cache',
  '.tox',
  '.eggs',
  '*.egg-info',
  'dist',
  'build',
  '.idea',
  '.vscode',
  '.DS_Store',
  '*.pyc',
  '*.pyo',
  '*.pyd',
  '.Python',
  'pip-log.txt',
  'pip-delete-this-directory.txt',
  '.coverage',
  'htmlcov',
  '.gradle',
  'target',
  'bin',
  'obj',
].map(v => ({ label: v, value: v })))

const backendOptions = [
  { label: '默认配置', value: 'default' },
  { label: '自动回退（ACE → fast-context）', value: 'auto' },
  { label: '仅 ACE / Augment', value: 'ace' },
  { label: '仅 fast-context', value: 'fast_context' },
  { label: '双后端合并', value: 'both' },
]

const autoOrderOptions = [
  { label: 'ACE / Augment', value: 'ace' },
  { label: 'fast-context', value: 'fast_context' },
]

const backendConfigOptions = backendOptions.filter(item => item.value !== 'default')

const selectedExtensionSet = computed(() => new Set(config.value.text_extensions || []))
const selectedPresetCount = computed(() =>
  allPresetExtensions.filter(ext => selectedExtensionSet.value.has(ext)).length,
)
const customExtensions = computed(() =>
  (config.value.text_extensions || []).filter(ext => !presetExtensionSet.has(ext)),
)
const filteredExtensionGroups = computed(() => {
  const keyword = extensionSearchQuery.value.trim().toLowerCase()
  if (!keyword) {
    return extensionGroups
  }

  return extensionGroups
    .map(group => ({
      ...group,
      extensions: group.extensions.filter(ext => ext.includes(keyword)),
    }))
    .filter(group => group.extensions.length > 0)
})

const aceEnabledInStrategy = computed(() => {
  const backend = config.value.sou_default_backend
  return backend === 'ace'
    || backend === 'both'
    || (backend === 'auto' && config.value.sou_auto_order.includes('ace'))
})

const fastContextEnabledInStrategy = computed(() => {
  const backend = config.value.sou_default_backend
  return backend === 'fast_context'
    || backend === 'both'
    || (backend === 'auto' && config.value.sou_auto_order.includes('fast_context'))
})

const backendStrategySummary = computed(() => {
  switch (config.value.sou_default_backend) {
    case 'ace':
      return '当前默认仅使用 ACE / Augment 后端。'
    case 'fast_context':
      return '当前默认仅使用 fast-context，ACE 连接配置可留空。'
    case 'both':
      return '当前默认同时返回 ACE 与 fast-context 的合并结果。'
    default:
      return '当前默认 ACE 优先，ACE 失败或索引不可用时自动切换到 fast-context。'
  }
})

type DebugStatusTone = 'neutral' | 'info' | 'success' | 'warning' | 'danger'

interface DebugStatusItem {
  key: string
  label: string
  value: string
  detail: string
  icon: string
  tone: DebugStatusTone
}

const debugBackendLabel = computed(() =>
  backendOptions.find(item => item.value === debugBackend.value)?.label || '默认配置',
)

const debugCanRun = computed(() =>
  debugProjectRoot.value.trim().length > 0 && debugQuery.value.trim().length > 0,
)

// 中文说明：顶部状态栏集中展示运行前最关键的环境信息，减少用户在多个区块之间来回判断。
const debugStatusItems = computed<DebugStatusItem[]>(() => {
  const lastResult = debugResultData.value
  const lastDebugTone: DebugStatusTone = lastResult
    ? (lastResult.success ? 'success' : 'danger')
    : 'neutral'

  return [
    {
      key: 'backend',
      label: '调试后端',
      value: debugBackendLabel.value,
      detail: debugBackend.value === 'default' ? '跟随默认策略' : '本次运行指定',
      icon: 'i-carbon-assembly-cluster',
      tone: debugBackend.value === 'default' ? 'info' : 'success',
    },
    {
      key: 'proxy',
      label: '代理',
      value: config.value.proxy_enabled ? '已启用' : '未启用',
      detail: config.value.proxy_enabled
        ? `${config.value.proxy_type.toUpperCase()} ${config.value.proxy_host}:${config.value.proxy_port}`
        : '直连或系统默认网络',
      icon: config.value.proxy_enabled ? 'i-carbon-connection-signal' : 'i-carbon-direct-link',
      tone: config.value.proxy_enabled ? 'success' : 'neutral',
    },
    {
      key: 'projects',
      label: '索引项目',
      value: `${debugProjectOptions.value.length} 个`,
      detail: debugProjectOptionsLoading.value
        ? '正在刷新项目列表'
        : (debugProjectOptions.value.length > 0 ? '可直接选择调试' : '聚焦项目框加载'),
      icon: 'i-carbon-folder-shared',
      tone: debugProjectOptions.value.length > 0 ? 'success' : 'warning',
    },
    {
      key: 'last-debug',
      label: '上次调试',
      value: lastResult ? (lastResult.success ? '成功' : '失败') : '未执行',
      detail: lastResult
        ? `${lastResult.total_duration_ms}ms · ${lastResult.result_count ?? '-'} 结果`
        : '等待运行',
      icon: lastResult
        ? (lastResult.success ? 'i-carbon-checkmark-filled' : 'i-carbon-warning-alt')
        : 'i-carbon-pending',
      tone: lastDebugTone,
    },
  ]
})

const debugResultTagType = computed<'success' | 'error'>(() =>
  debugResultData.value?.success ? 'success' : 'error',
)

const debugResultIcon = computed(() =>
  debugResultData.value?.success ? 'i-carbon-checkmark-filled' : 'i-carbon-warning-alt',
)

const debugResultTitle = computed(() =>
  debugResultData.value?.success ? '调试成功' : '调试失败',
)

const debugResultSubtitle = computed(() => {
  if (!debugResultData.value)
    return ''

  return `${formatDebugTime(debugResultData.value.response_time)} · ${debugBackendLabel.value}`
})

// --- 操作函数 ---

function normalizeBaseUrl(value: string): string {
  return (value || '').trim().replace(/\/+$/, '')
}

async function loadAcemcpConfig() {
  loadingConfig.value = true
  try {
    const res = await invoke('get_acemcp_config') as any

    config.value = {
      base_url: res.base_url || '',
      token: res.token || '',
      batch_size: res.batch_size,
      max_lines_per_blob: res.max_lines_per_blob,
      text_extensions: res.text_extensions,
      exclude_patterns: res.exclude_patterns,
      watch_debounce_minutes: Math.round((res.watch_debounce_ms || 180000) / 60000),
      // 代理配置
      proxy_enabled: res.proxy_enabled || false,
      proxy_host: res.proxy_host || '127.0.0.1',
      proxy_port: res.proxy_port || 7890,
      proxy_type: res.proxy_type || 'http',
      proxy_username: res.proxy_username || '',
      proxy_password: res.proxy_password || '',
      // 嵌套项目索引配置
      index_nested_projects: res.index_nested_projects ?? true,
      // sou 多后端配置
      sou_default_backend: res.sou_default_backend || 'auto',
      sou_auto_order: res.sou_auto_order || ['ace', 'fast_context'],
      sou_include_backend_headers: res.sou_include_backend_headers ?? true,
      sou_include_failed_backend_errors: res.sou_include_failed_backend_errors ?? true,
      // fast-context 配置
      fast_context_api_key: res.fast_context_api_key || '',
      fast_context_tree_depth: res.fast_context_tree_depth || 3,
      fast_context_max_turns: res.fast_context_max_turns || 3,
      fast_context_max_results: res.fast_context_max_results || 10,
      fast_context_max_commands: res.fast_context_max_commands || 8,
      fast_context_timeout_ms: res.fast_context_timeout_ms || 30000,
      fast_context_exclude_paths: res.fast_context_exclude_paths || ['node_modules', '.git', 'dist', 'build', 'target'],
    }
    if (!config.value.fast_context_api_key) {
      // 中文说明：配置页首次加载时主动尝试读取并保存 Windsurf API Key，失败时保留手动填写入口。
      await detectFastContextApiKey(false)
    }
    lastSavedConnection.value = {
      base_url: normalizeBaseUrl(res.base_url || ''),
      token: (res.token || '').trim(),
    }

    // 确保选项存在
    const extSet = new Set(extOptions.value.map(o => o.value))
    for (const v of config.value.text_extensions) {
      if (!extSet.has(v)) {
        extOptions.value.push({ label: v, value: v })
      }
    }
    const exSet = new Set(excludeOptions.value.map(o => o.value))
    for (const v of config.value.exclude_patterns) {
      if (!exSet.has(v)) {
        excludeOptions.value.push({ label: v, value: v })
      }
    }
  }
  catch (err) {
    message.error(`加载配置失败: ${err}`)
  }
  finally {
    loadingConfig.value = false
  }
}

async function detectFastContextApiKey(showFeedback = true) {
  detectingFastContextKey.value = true
  fastContextKeyStatus.value = ''
  fastContextKeyStatusType.value = 'info'
  try {
    const result = await invoke<FastContextApiKeyDetectionResult>('detect_fast_context_api_key', {
      save: true,
      includeSaved: !showFeedback,
    })

    fastContextKeyStatus.value = result.message
    if (result.found && result.api_key) {
      config.value.fast_context_api_key = result.api_key
      fastContextKeyStatusType.value = 'success'
      if (showFeedback) {
        message.success(result.message)
      }
    }
    else {
      fastContextKeyStatusType.value = 'warning'
      if (showFeedback) {
        message.warning(result.message || '未获取到 Windsurf API Key，请手动填写')
      }
    }
  }
  catch (err) {
    const msg = `获取 Windsurf API Key 失败: ${err}`
    fastContextKeyStatus.value = msg
    fastContextKeyStatusType.value = 'error'
    if (showFeedback) {
      message.error(msg)
    }
  }
  finally {
    detectingFastContextKey.value = false
  }
}

async function saveConfig() {
  try {
    if (config.value.base_url && !/^https?:\/\//i.test(config.value.base_url)) {
      message.error('URL无效，需以 http(s):// 开头；如只使用 fast-context，可留空')
      return
    }

    // 支持用户直接粘贴完整代理地址（http(s)/socks5://user:pass@host:port）
    // 避免将完整 URL 误填入“代理地址(host)”导致后端拼接出无效代理 URL
    const proxyInput = (config.value.proxy_host || '').trim()
    if (proxyInput.includes('://')) {
      try {
        const u = new URL(proxyInput)
        const scheme = (u.protocol || '').replace(':', '')
        if (!['http', 'https', 'socks5'].includes(scheme)) {
          message.error('代理地址协议不支持，仅支持 http/https/socks5')
          return
        }

        config.value.proxy_type = scheme as 'http' | 'https' | 'socks5'
        config.value.proxy_host = u.hostname
        if (u.port) {
          config.value.proxy_port = Number(u.port)
        }
        if (u.username) {
          config.value.proxy_username = decodeURIComponent(u.username)
        }
        if (u.password) {
          config.value.proxy_password = decodeURIComponent(u.password)
        }
      }
      catch (e) {
        message.error(`代理地址格式无效: ${String(e)}`)
        return
      }
    }

    const nextBaseUrl = normalizeBaseUrl(config.value.base_url)
    const nextToken = (config.value.token || '').trim()
    const connectionChanged = lastSavedConnection.value.base_url !== nextBaseUrl
      || lastSavedConnection.value.token !== nextToken

    await invoke('save_acemcp_config', {
      args: {
        baseUrl: config.value.base_url,
        token: config.value.token,
        batchSize: config.value.batch_size,
        maxLinesPerBlob: config.value.max_lines_per_blob,
        textExtensions: config.value.text_extensions,
        excludePatterns: config.value.exclude_patterns,
        watchDebounceMs: config.value.watch_debounce_minutes * 60000,
        // 代理配置
        proxyEnabled: config.value.proxy_enabled,
        proxyHost: config.value.proxy_host,
        proxyPort: config.value.proxy_port,
        proxyType: config.value.proxy_type,
        proxyUsername: config.value.proxy_username,
        proxyPassword: config.value.proxy_password,
        // 嵌套项目索引配置
        indexNestedProjects: config.value.index_nested_projects,
        // sou 多后端配置
        souDefaultBackend: config.value.sou_default_backend,
        souAutoOrder: config.value.sou_auto_order,
        souIncludeBackendHeaders: config.value.sou_include_backend_headers,
        souIncludeFailedBackendErrors: config.value.sou_include_failed_backend_errors,
        // fast-context 配置
        fastContextApiKey: config.value.fast_context_api_key,
        fastContextTreeDepth: config.value.fast_context_tree_depth,
        fastContextMaxTurns: config.value.fast_context_max_turns,
        fastContextMaxResults: config.value.fast_context_max_results,
        fastContextMaxCommands: config.value.fast_context_max_commands,
        fastContextTimeoutMs: config.value.fast_context_timeout_ms,
        fastContextExcludePaths: config.value.fast_context_exclude_paths,
      },
    })
    lastSavedConnection.value = {
      base_url: nextBaseUrl,
      token: nextToken,
    }
    message.success('配置已保存')
    if (connectionChanged) {
      message.warning('检测到 ACE 配置变更，现有索引将在下次搜索时自动重建', {
        duration: 5000,
      })
    }
  }
  catch (err) {
    message.error(`保存失败: ${err}`)
  }
}

async function testConnection() {
  const loadingMsg = message.loading('正在测试连接...', { duration: 0 })
  try {
    const result = await invoke('test_acemcp_connection', {
      args: {
        baseUrl: config.value.base_url,
        token: config.value.token,
      },
    }) as {
      success: boolean
      message: string
    }

    if (result.success) {
      message.success(result.message)
    }
    else {
      message.error(result.message)
    }
  }
  catch (err) {
    message.error(`连接测试失败: ${err}`)
  }
  finally {
    loadingMsg.destroy()
  }
}

/** 加载调试用项目选择列表 */
async function loadDebugProjectOptions() {
  debugProjectOptionsLoading.value = true
  try {
    const statusResult = await invoke<{ projects: Record<string, { project_root: string, total_files: number }> }>('get_all_acemcp_index_status')
    const list = Object.values(statusResult.projects || {})
      .filter(p => (p.total_files || 0) > 0)
      .map(p => ({
        label: `${getProjectName(p.project_root)} (${p.total_files} 文件)`,
        value: p.project_root,
      }))
    debugProjectOptions.value = list
    // 如果列表不为空且当前未选择项目，自动选择第一个
    if (list.length > 0 && !debugProjectRoot.value) {
      debugProjectRoot.value = list[0].value
    }
  }
  catch (e) {
    console.error('加载项目列表失败:', e)
    debugProjectOptions.value = []
  }
  finally {
    debugProjectOptionsLoading.value = false
  }
}

async function runToolDebug() {
  if (!debugProjectRoot.value || !debugQuery.value) {
    message.warning('请填写项目路径和查询语句')
    return
  }

  debugLoading.value = true
  debugResultData.value = null

  try {
    const result = await invoke<DebugSearchResult>('debug_acemcp_search', {
      projectRootPath: debugProjectRoot.value,
      query: debugQuery.value,
      backend: debugBackend.value,
    })

    debugResultData.value = result

    if (result.success) {
      message.success(`调试执行成功，耗时 ${result.total_duration_ms}ms`)
    }
    else {
      message.error(result.error || '调试失败')
    }
  }
  catch (e: any) {
    const msg = e?.message || String(e)
    // 创建错误结果
    debugResultData.value = {
      success: false,
      error: msg,
      request_time: new Date().toISOString(),
      response_time: new Date().toISOString(),
      total_duration_ms: 0,
      project_path: debugProjectRoot.value,
      query: debugQuery.value,
    }
    message.error(`调试异常: ${msg}`)
  }
  finally {
    debugLoading.value = false
  }
}

/** 格式化调试时间显示 */
function formatDebugTime(isoTime: string): string {
  try {
    const date = new Date(isoTime)
    return date.toLocaleString('zh-CN', {
      hour: '2-digit',
      minute: '2-digit',
      second: '2-digit',
      fractionalSecondDigits: 3,
    })
  }
  catch {
    return isoTime
  }
}

/** 复制调试结果到剪贴板 */
async function copyDebugResult() {
  if (!debugResultData.value?.result) {
    message.warning('没有可复制的内容')
    return
  }
  try {
    await navigator.clipboard.writeText(debugResultData.value.result)
    message.success('已复制到剪贴板')
  }
  catch (e) {
    message.error(`复制失败: ${e}`)
  }
}

async function clearCache() {
  try {
    message.loading('正在清除...')
    const res = await invoke('clear_acemcp_cache') as string
    message.success(res)
  }
  catch (e) {
    message.error(`清除失败: ${e}`)
  }
}

async function toggleAutoIndex() {
  try {
    await setAutoIndexEnabled(!autoIndexEnabled.value)
    message.success(`自动索引已${autoIndexEnabled.value ? '启用' : '禁用'}`)
  }
  catch (e) {
    message.error(String(e))
  }
}

// --- 代理检测和测速函数移至 ProxySettingsModal ---

function getProjectName(projectRoot: string): string {
  const parts = (projectRoot || '').replace(/\\/g, '/').split('/').filter(Boolean)
  return parts.length > 0 ? parts[parts.length - 1] : projectRoot
}

function normalizeExtensionList(list: string[]): string[] {
  return Array.from(new Set((list || []).map((s) => {
    const t = String(s || '').trim().toLowerCase()
    return t ? (t.startsWith('.') ? t : `.${t}`) : ''
  }).filter(Boolean)))
}

function setExtensions(list: string[]) {
  config.value.text_extensions = normalizeExtensionList(list)
}

function addExtensions(list: string[]) {
  setExtensions([...(config.value.text_extensions || []), ...list])
}

function addExtensionGroup(group: ExtensionGroup) {
  addExtensions(group.extensions)
  message.success(`已加入${group.label}扩展名`)
}

function applyExtensionPreset(preset: ExtensionPreset) {
  setExtensions(preset.extensions)
  message.success(`已应用${preset.label}模板`)
}

function addAllPresetExtensions() {
  addExtensions(allPresetExtensions)
  message.success('已加入全部预设扩展名')
}

function clearExtensions() {
  config.value.text_extensions = []
}

function toggleExtension(ext: string) {
  const normalized = normalizeExtensionList([ext])[0]
  if (!normalized) {
    return
  }

  const current = selectedExtensionSet.value
  if (current.has(normalized)) {
    setExtensions((config.value.text_extensions || []).filter(item => item !== normalized))
  }
  else {
    addExtensions([normalized])
  }
}

function invertExtensionGroup(group: ExtensionGroup) {
  const current = selectedExtensionSet.value
  const next = [
    ...(config.value.text_extensions || []).filter(ext => !group.extensions.includes(ext)),
    ...group.extensions.filter(ext => !current.has(ext)),
  ]
  setExtensions(next)
}

function keepOnlyExtensionGroup(group: ExtensionGroup) {
  setExtensions(group.extensions)
  message.success(`已仅保留${group.label}扩展名`)
}

function removeExtensionGroup(group: ExtensionGroup) {
  setExtensions((config.value.text_extensions || []).filter(ext => !group.extensions.includes(ext)))
  message.success(`已移除${group.label}扩展名`)
}

function handleExtensionGroupAction(key: string, group: ExtensionGroup) {
  if (key === 'invert') {
    invertExtensionGroup(group)
  }
  else if (key === 'keep') {
    keepOnlyExtensionGroup(group)
  }
  else if (key === 'remove') {
    removeExtensionGroup(group)
  }
}

function groupSelectedCount(group: ExtensionGroup): number {
  const current = selectedExtensionSet.value
  return group.extensions.filter(ext => current.has(ext)).length
}

// 监听扩展名变化，自动规范化
watch(() => config.value.text_extensions, (list) => {
  const norm = normalizeExtensionList(list || [])

  if (norm.join(',') !== list.join(',')) {
    config.value.text_extensions = norm
  }
}, { deep: true })

// 组件挂载
onMounted(async () => {
  if (props.active) {
    await loadAcemcpConfig()
    await Promise.all([
      fetchAutoIndexEnabled(),
      fetchWatchingProjects(),
    ])
  }
})

defineExpose({ saveConfig })
</script>

<template>
  <div class="sou-config">
    <n-tabs type="line" animated>
      <!-- 基础配置 -->
      <n-tab-pane name="basic" tab="基础配置">
        <n-scrollbar class="tab-scrollbar">
          <n-space vertical size="large" class="tab-content">
            <ConfigSection title="ACE 连接设置" :description="aceEnabledInStrategy ? '配置 ACE / Augment 索引服务连接信息' : '当前默认策略不依赖 ACE，以下配置可留空'">
              <n-alert type="info" :bordered="false" class="mb-4">
                {{ backendStrategySummary }}
              </n-alert>
              <n-grid :x-gap="24" :y-gap="16" :cols="1">
                <n-grid-item>
                  <n-form-item label="API端点URL">
                    <n-input
                      v-model:value="config.base_url"
                      placeholder="https://api.example.com"
                      clearable
                      :disabled="!aceEnabledInStrategy"
                    />
                    <template #feedback>
                      <span class="form-feedback">
                        {{ aceEnabledInStrategy ? 'ACE 后端需要配置 API 端点。' : 'fast-context-only 时可以留空。' }}
                      </span>
                    </template>
                  </n-form-item>
                </n-grid-item>
                <n-grid-item>
                  <n-form-item label="认证令牌">
                    <n-input
                      v-model:value="config.token"
                      type="password"
                      show-password-on="click"
                      placeholder="输入认证令牌"
                      clearable
                      :disabled="!aceEnabledInStrategy"
                    />
                    <template #feedback>
                      <span class="form-feedback">
                        {{ aceEnabledInStrategy ? '用于 ACE / Augment 的 Bearer Token。' : '当前默认不使用 ACE Token。' }}
                      </span>
                    </template>
                  </n-form-item>
                </n-grid-item>
              </n-grid>
            </ConfigSection>

            <ConfigSection title="性能参数" description="调整处理批量和文件大小限制">
              <n-grid :x-gap="24" :cols="2">
                <n-grid-item>
                  <n-form-item label="批处理大小">
                    <n-input-number v-model:value="config.batch_size" :min="1" :max="100" class="w-full" />
                  </n-form-item>
                </n-grid-item>
                <n-grid-item>
                  <n-form-item label="最大行数/块">
                    <n-input-number v-model:value="config.max_lines_per_blob" :min="100" :max="5000" class="w-full" />
                  </n-form-item>
                </n-grid-item>
              </n-grid>
            </ConfigSection>

            <!-- 代理设置 -->
            <!-- 代理设置（重构后的简化卡片） -->
            <ConfigSection title="代理设置" description="配置 HTTP/HTTPS 代理以优化网络连接及访问速度">
              <div class="flex items-center justify-between p-4 rounded-xl border border-slate-200 dark:border-slate-700 bg-white dark:bg-slate-800/50">
                <div class="flex items-center gap-4">
                  <!-- 状态图标 -->
                  <div
                    class="w-10 h-10 rounded-full flex items-center justify-center transition-colors"
                    :class="config.proxy_enabled ? 'bg-green-100 text-green-600 dark:bg-green-900/30 dark:text-green-400' : 'bg-slate-100 text-slate-400 dark:bg-slate-800 dark:text-slate-500'"
                  >
                    <div class="i-carbon-network-3 text-xl" />
                  </div>

                  <div>
                    <div class="font-medium text-base flex items-center gap-2">
                      代理服务
                      <n-tag :type="config.proxy_enabled ? 'success' : 'default'" size="small" :bordered="false">
                        {{ config.proxy_enabled ? '已启用' : '未启用' }}
                      </n-tag>
                    </div>
                    <div class="text-xs text-gray-500 mt-0.5">
                      <span v-if="config.proxy_enabled">
                        {{ config.proxy_type.toUpperCase() }}://{{ config.proxy_host }}:{{ config.proxy_port }}
                      </span>
                      <span v-else>
                        当前使用直连模式，配置代理可加速海外 API 访问
                      </span>
                    </div>
                  </div>
                </div>

                <n-button secondary @click="showProxyModal = true">
                  <template #icon>
                    <div class="i-carbon-settings-adjust" />
                  </template>
                  配置代理与诊断
                </n-button>
              </div>
            </ConfigSection>

            <div class="flex justify-end">
              <n-button type="primary" @click="saveConfig">
                <template #icon>
                  <div class="i-carbon-save" />
                </template>
                保存配置
              </n-button>
            </div>
          </n-space>
        </n-scrollbar>
      </n-tab-pane>

      <!-- 后端切换 -->
      <n-tab-pane name="backend" tab="后端切换">
        <n-scrollbar class="tab-scrollbar">
          <n-space vertical size="large" class="tab-content">
            <ConfigSection title="切换策略" description="配置默认后端、主动切换入口和双后端合并返回">
              <n-space vertical size="medium">
                <n-alert type="success" :bordered="false">
                  推荐默认策略：ACE 优先，失败或索引不可用时自动切换到 fast-context。
                  MCP 调用时也可以通过 <code>backend</code> 主动指定 ace、fast_context 或 both。
                </n-alert>

                <n-grid :x-gap="24" :y-gap="16" :cols="2">
                  <n-grid-item>
                    <n-form-item label="默认策略">
                      <n-select
                        v-model:value="config.sou_default_backend"
                        :options="backendConfigOptions"
                      />
                      <template #feedback>
                        <span class="form-feedback">{{ backendStrategySummary }}</span>
                      </template>
                    </n-form-item>
                  </n-grid-item>
                  <n-grid-item>
                    <n-form-item label="自动模式优先级">
                      <n-select
                        v-model:value="config.sou_auto_order"
                        :options="autoOrderOptions"
                        multiple
                        :max-tag-count="2"
                      />
                      <template #feedback>
                        <span class="form-feedback">默认保持 ACE 在前，ACE 失败后切 fast-context。</span>
                      </template>
                    </n-form-item>
                  </n-grid-item>
                </n-grid>

                <n-grid :x-gap="24" :y-gap="16" :cols="3">
                  <n-grid-item>
                    <n-form-item label="启用 ACE">
                      <n-tag :type="aceEnabledInStrategy ? 'success' : 'default'" :bordered="false">
                        {{ aceEnabledInStrategy ? '会参与检索' : '默认不参与' }}
                      </n-tag>
                    </n-form-item>
                  </n-grid-item>
                  <n-grid-item>
                    <n-form-item label="启用 fast-context">
                      <n-tag :type="fastContextEnabledInStrategy ? 'success' : 'default'" :bordered="false">
                        {{ fastContextEnabledInStrategy ? '会参与检索' : '默认不参与' }}
                      </n-tag>
                    </n-form-item>
                  </n-grid-item>
                  <n-grid-item>
                    <n-form-item label="主动切换调试">
                      <n-select
                        v-model:value="debugBackend"
                        :options="backendOptions"
                      />
                    </n-form-item>
                  </n-grid-item>
                </n-grid>

                <n-grid :x-gap="24" :y-gap="16" :cols="2">
                  <n-grid-item>
                    <n-form-item label="显示后端来源">
                      <n-switch v-model:value="config.sou_include_backend_headers" />
                    </n-form-item>
                  </n-grid-item>
                  <n-grid-item>
                    <n-form-item label="保留失败诊断">
                      <n-switch v-model:value="config.sou_include_failed_backend_errors" />
                    </n-form-item>
                  </n-grid-item>
                </n-grid>
              </n-space>
            </ConfigSection>

            <ConfigSection title="Fast Context" description="Rust 原生 fast-context，无需 Node bridge；仅配置 Windsurf 搜索参数">
              <n-space vertical size="medium">
                <n-form-item label="Windsurf API Key">
                  <n-space vertical size="small" class="w-full">
                    <n-input-group>
                      <n-button
                        secondary
                        type="primary"
                        :loading="detectingFastContextKey"
                        @click="detectFastContextApiKey(true)"
                      >
                        手动获取
                      </n-button>
                      <n-input
                        v-model:value="config.fast_context_api_key"
                        type="password"
                        show-password-on="click"
                        placeholder="自动读取失败时请手动填写"
                        clearable
                      />
                    </n-input-group>
                    <n-alert
                      v-if="fastContextKeyStatus"
                      :type="fastContextKeyStatusType"
                      :bordered="false"
                      class="compact-alert"
                    >
                      {{ fastContextKeyStatus }}
                    </n-alert>
                  </n-space>
                </n-form-item>

                <n-grid :x-gap="24" :y-gap="16" :cols="4">
                  <n-grid-item>
                    <n-form-item label="tree depth">
                      <n-input-number v-model:value="config.fast_context_tree_depth" :min="1" :max="6" class="w-full" />
                    </n-form-item>
                  </n-grid-item>
                  <n-grid-item>
                    <n-form-item label="max turns">
                      <n-input-number v-model:value="config.fast_context_max_turns" :min="1" :max="5" class="w-full" />
                    </n-form-item>
                  </n-grid-item>
                  <n-grid-item>
                    <n-form-item label="max results">
                      <n-input-number v-model:value="config.fast_context_max_results" :min="1" :max="30" class="w-full" />
                    </n-form-item>
                  </n-grid-item>
                  <n-grid-item>
                    <n-form-item label="timeout">
                      <n-input-number
                        v-model:value="config.fast_context_timeout_ms"
                        :min="1000"
                        :max="300000"
                        :step="1000"
                        class="w-full"
                      />
                    </n-form-item>
                  </n-grid-item>
                </n-grid>

                <n-form-item label="fast-context 排除路径">
                  <n-select
                    v-model:value="config.fast_context_exclude_paths"
                    :options="excludeOptions"
                    multiple
                    tag
                    filterable
                    clearable
                    placeholder="node_modules / target / dist ..."
                  />
                </n-form-item>

                <div class="flex justify-end">
                  <n-button type="primary" @click="saveConfig">
                    <template #icon>
                      <div class="i-carbon-save" />
                    </template>
                    保存后端配置
                  </n-button>
                </div>
              </n-space>
            </ConfigSection>
          </n-space>
        </n-scrollbar>
      </n-tab-pane>

      <!-- 高级配置 -->
      <n-tab-pane name="advanced" tab="高级配置">
        <n-scrollbar class="tab-scrollbar">
          <n-space vertical size="large" class="tab-content">
            <ConfigSection title="文件过滤" description="设置需索引的文件类型和排除规则">
              <n-space vertical size="medium">
                <n-form-item label="包含扩展名">
                  <div class="extension-manager">
                    <div class="extension-manager-head">
                      <div class="extension-stats">
                        <n-tag type="success" size="small" :bordered="false">
                          已选 {{ config.text_extensions.length }}
                        </n-tag>
                        <n-tag size="small" :bordered="false">
                          预设 {{ selectedPresetCount }}/{{ allPresetExtensions.length }}
                        </n-tag>
                        <n-tag v-if="customExtensions.length" type="info" size="small" :bordered="false">
                          自定义 {{ customExtensions.length }}
                        </n-tag>
                      </div>
                      <div class="extension-actions">
                        <n-button size="tiny" secondary @click="addAllPresetExtensions">
                          <template #icon>
                            <div class="i-carbon-add-alt" />
                          </template>
                          加入全部
                        </n-button>
                        <n-button size="tiny" quaternary @click="clearExtensions">
                          <template #icon>
                            <div class="i-carbon-trash-can" />
                          </template>
                          清空
                        </n-button>
                      </div>
                    </div>

                    <n-select
                      v-model:value="config.text_extensions"
                      :options="extOptions"
                      multiple tag filterable clearable
                      :max-tag-count="12"
                      placeholder="输入或选择扩展名 (.vue)"
                    />

                    <div class="extension-presets">
                      <button
                        v-for="preset in extensionPresets"
                        :key="preset.label"
                        type="button"
                        class="extension-preset"
                        @click="applyExtensionPreset(preset)"
                      >
                        <div class="extension-preset-icon" :class="preset.icon" />
                        <div class="extension-preset-copy">
                          <div class="extension-preset-title">
                            {{ preset.label }}
                          </div>
                          <div class="extension-preset-desc">
                            {{ preset.description }}
                          </div>
                        </div>
                      </button>
                    </div>

                    <div class="extension-toolbar">
                      <n-input
                        v-model:value="extensionSearchQuery"
                        size="small"
                        clearable
                        placeholder="搜索预设扩展名"
                        class="extension-search"
                      >
                        <template #prefix>
                          <div class="i-carbon-search text-xs opacity-60" />
                        </template>
                      </n-input>
                    </div>

                    <div class="extension-groups">
                      <div
                        v-for="group in filteredExtensionGroups"
                        :key="group.id"
                        class="extension-group"
                      >
                        <div class="extension-group-header">
                          <div class="extension-group-title">
                            <div class="extension-group-icon" :class="group.icon" />
                            <div>
                              <div class="extension-group-name">
                                {{ group.label }}
                              </div>
                              <div class="extension-group-desc">
                                {{ group.description }}
                              </div>
                            </div>
                          </div>
                          <div class="extension-group-actions">
                            <n-button size="tiny" secondary @click="addExtensionGroup(group)">
                              补齐 {{ groupSelectedCount(group) }}/{{ group.extensions.length }}
                            </n-button>
                            <n-dropdown
                              trigger="click"
                              :options="extensionGroupActionOptions"
                              @select="key => handleExtensionGroupAction(String(key), group)"
                            >
                              <n-button size="tiny" quaternary circle>
                                <template #icon>
                                  <div class="i-carbon-overflow-menu-horizontal" />
                                </template>
                              </n-button>
                            </n-dropdown>
                          </div>
                        </div>

                        <div class="extension-chip-row">
                          <button
                            v-for="ext in group.extensions"
                            :key="ext"
                            type="button"
                            class="extension-chip"
                            :class="{ 'extension-chip-selected': selectedExtensionSet.has(ext) }"
                            @click="toggleExtension(ext)"
                          >
                            {{ ext }}
                          </button>
                        </div>
                      </div>
                    </div>

                    <div v-if="customExtensions.length" class="custom-extension-row">
                      <span class="custom-extension-label">自定义</span>
                      <button
                        v-for="ext in customExtensions"
                        :key="ext"
                        type="button"
                        class="extension-chip extension-chip-selected"
                        @click="toggleExtension(ext)"
                      >
                        {{ ext }}
                      </button>
                    </div>
                  </div>
                  <template #feedback>
                    <span class="form-feedback">小写，点开头，自动去重；点击标签可快速加入或移除</span>
                  </template>
                </n-form-item>

                <n-form-item label="排除模式">
                  <n-select
                    v-model:value="config.exclude_patterns"
                    :options="excludeOptions"
                    multiple tag filterable clearable
                    placeholder="输入或选择排除模式 (node_modules)"
                  />
                  <template #feedback>
                    <span class="form-feedback">
                      支持 glob 通配符
                    </span>
                  </template>
                </n-form-item>
              </n-space>
            </ConfigSection>

            <div class="flex justify-end">
              <n-button type="primary" @click="saveConfig">
                <template #icon>
                  <div class="i-carbon-save" />
                </template>
                保存配置
              </n-button>
            </div>
          </n-space>
        </n-scrollbar>
      </n-tab-pane>

      <!-- Sou 专属调试 -->
      <n-tab-pane name="debug" tab="调试">
        <n-scrollbar class="tab-scrollbar">
          <div class="tab-content debug-tab">
            <section class="debug-topbar">
              <div class="debug-topbar-main">
                <div class="debug-eyebrow">
                  <div class="i-carbon-terminal" />
                  Sou 调试工作台
                </div>
                <div class="debug-status-grid">
                  <div
                    v-for="item in debugStatusItems"
                    :key="item.key"
                    class="debug-status-pill"
                    :class="`debug-status-pill--${item.tone}`"
                  >
                    <div class="debug-status-icon">
                      <div :class="item.icon" />
                    </div>
                    <div class="debug-status-copy">
                      <div class="debug-status-label">
                        {{ item.label }}
                      </div>
                      <div class="debug-status-value">
                        {{ item.value }}
                      </div>
                      <div class="debug-status-detail">
                        {{ item.detail }}
                      </div>
                    </div>
                  </div>
                </div>
              </div>
              <div class="debug-topbar-actions">
                <n-button size="small" secondary :disabled="!aceEnabledInStrategy" @click="testConnection">
                  <template #icon>
                    <div class="i-carbon-connection-signal" />
                  </template>
                  测试 ACE
                </n-button>
                <n-button size="small" tertiary @click="clearCache">
                  <template #icon>
                    <div class="i-carbon-clean" />
                  </template>
                  清缓存
                </n-button>
              </div>
            </section>

            <n-alert type="info" :bordered="false" class="info-alert debug-note">
              <template #icon>
                <div class="i-carbon-information" />
              </template>
              全局日志已迁移到主界面的“日志”页；此处仅保留 Sou / ACE 连接、缓存和搜索调试。
            </n-alert>

            <ConfigSection title="搜索调试" description="模拟 Sou 搜索请求，验证后端策略、项目索引和响应内容。">
              <div class="debug-console">
                <div class="debug-console-row debug-console-row--controls">
                  <n-form-item :show-feedback="false" class="debug-field debug-field--project">
                    <template #label>
                      <div class="debug-field-label">
                        <span>项目路径</span>
                        <n-button
                          text
                          size="tiny"
                          type="primary"
                          @click="debugUseManualInput = !debugUseManualInput"
                        >
                          {{ debugUseManualInput ? '选择已索引' : '手动输入' }}
                        </n-button>
                      </div>
                    </template>
                    <n-select
                      v-if="!debugUseManualInput"
                      v-model:value="debugProjectRoot"
                      :options="debugProjectOptions"
                      :loading="debugProjectOptionsLoading"
                      placeholder="选择已索引的项目..."
                      filterable
                      clearable
                      @focus="loadDebugProjectOptions"
                    />
                    <n-input
                      v-else
                      v-model:value="debugProjectRoot"
                      placeholder="/abs/path/to/project"
                      clearable
                    />
                  </n-form-item>

                  <n-form-item label="搜索后端" :show-feedback="false" class="debug-field debug-field--backend">
                    <n-select
                      v-model:value="debugBackend"
                      :options="backendOptions"
                    />
                  </n-form-item>
                </div>

                <div class="debug-console-row debug-console-row--query">
                  <n-form-item label="查询语句" :show-feedback="false" class="debug-field debug-field--query">
                    <n-input
                      v-model:value="debugQuery"
                      type="textarea"
                      :rows="3"
                      placeholder="输入搜索意图，例如：SouConfig 调试 tab 状态栏"
                    />
                  </n-form-item>

                  <div class="debug-run-panel">
                    <n-button
                      type="primary"
                      size="large"
                      :loading="debugLoading"
                      :disabled="!debugCanRun"
                      class="debug-run-button"
                      @click="runToolDebug"
                    >
                      <template #icon>
                        <div class="i-carbon-play" />
                      </template>
                      运行调试
                    </n-button>
                    <div class="debug-run-hint">
                      {{ debugCanRun ? '将按当前后端策略执行一次真实搜索' : '请先选择项目并填写查询语句' }}
                    </div>
                  </div>
                </div>
              </div>

              <!-- 骨架屏加载态 -->
              <div v-if="debugLoading" class="debug-skeleton">
                <div class="debug-skeleton-header">
                  <div class="i-carbon-in-progress animate-spin" />
                  正在请求 Sou 后端...
                </div>
                <n-skeleton text :repeat="3" />
                <n-skeleton text style="width: 60%" />
              </div>

              <!-- 结构化结果展示 -->
              <n-collapse-transition :show="debugResultData !== null && !debugLoading">
                <div v-if="debugResultData" class="debug-result-panel">
                  <div class="debug-result-header" :class="debugResultData.success ? 'is-success' : 'is-error'">
                    <div class="debug-result-title-group">
                      <div class="debug-result-icon">
                        <div :class="debugResultIcon" />
                      </div>
                      <div>
                        <div class="debug-result-title">
                          {{ debugResultTitle }}
                        </div>
                        <div class="debug-result-subtitle">
                          {{ debugResultSubtitle }}
                        </div>
                      </div>
                    </div>
                    <div class="debug-result-actions">
                      <n-tag :type="debugResultTagType" size="small" :bordered="false">
                        {{ debugResultData.success ? '成功' : '失败' }}
                      </n-tag>
                      <n-button
                        v-if="debugResultData.success && debugResultData.result"
                        size="tiny"
                        secondary
                        @click="copyDebugResult"
                      >
                        <template #icon>
                          <div class="i-carbon-copy" />
                        </template>
                        复制结果
                      </n-button>
                    </div>
                  </div>

                  <!-- 性能指标 -->
                  <div class="result-section result-section--metrics">
                    <div class="metric-item">
                      <div class="metric-value" :class="debugResultData.success ? 'text-emerald-500' : 'text-red-500'">
                        {{ debugResultData.total_duration_ms }}ms
                      </div>
                      <div class="metric-label">
                        总耗时
                      </div>
                    </div>
                    <div class="metric-item">
                      <div class="metric-value">
                        {{ debugResultData.result_count ?? '-' }}
                      </div>
                      <div class="metric-label">
                        结果数
                      </div>
                    </div>
                    <div class="metric-item">
                      <div class="metric-value metric-value--small">
                        {{ debugBackendLabel }}
                      </div>
                      <div class="metric-label">
                        本次后端
                      </div>
                    </div>
                  </div>

                  <!-- 请求信息 -->
                  <div class="result-section">
                    <div class="section-header">
                      <div class="i-carbon-send text-blue-500" />
                      <span>请求信息</span>
                    </div>
                    <div class="section-content debug-info-grid">
                      <div class="info-row">
                        <span class="info-label">项目</span>
                        <code class="info-value">{{ debugResultData.project_path }}</code>
                      </div>
                      <div class="info-row">
                        <span class="info-label">查询</span>
                        <span class="info-value">{{ debugResultData.query }}</span>
                      </div>
                      <div class="info-row">
                        <span class="info-label">发送时间</span>
                        <span class="info-value">{{ formatDebugTime(debugResultData.request_time) }}</span>
                      </div>
                    </div>
                  </div>

                  <!-- 响应数据 / 错误信息 -->
                  <div class="result-section">
                    <div class="section-header">
                      <div :class="debugResultData.success ? 'i-carbon-document text-emerald-500' : 'i-carbon-warning text-red-500'" />
                      <span>{{ debugResultData.success ? '响应数据' : '错误信息' }}</span>
                    </div>
                    <div class="section-content">
                      <n-alert
                        v-if="debugResultData.success"
                        type="info"
                        :bordered="false"
                        class="compact-alert mb-2"
                      >
                        fast-context 的 Path/Lines/L行号 是三术按 answer 文件范围本地读取后生成的 ACE 兼容格式，不是 fast-context 原生直出文本。
                      </n-alert>
                      <div v-if="debugResultData.error" class="error-content">
                        {{ debugResultData.error }}
                      </div>
                      <n-scrollbar v-else class="result-scrollbar">
                        <pre class="result-pre">{{ debugResultData.result || '无返回结果' }}</pre>
                      </n-scrollbar>
                    </div>
                  </div>
                </div>
              </n-collapse-transition>
            </ConfigSection>
          </div>
        </n-scrollbar>
      </n-tab-pane>

      <!-- 索引管理 -->
      <n-tab-pane name="index" tab="索引管理">
        <n-scrollbar class="tab-scrollbar">
          <n-space vertical size="large" class="tab-content">
            <ConfigSection title="全局策略">
              <n-alert type="info" :bordered="false" class="mb-3">
                监听索引的长期职责由三术 MCP 进程维护；此页面只记录监听项目并展示状态。等一下窗口关闭后，三术进程会按配置恢复监听。
              </n-alert>

              <div class="auto-index-toggle">
                <div class="toggle-info">
                  <div class="toggle-icon">
                    <div class="i-carbon-automatic w-5 h-5 text-primary-500" />
                  </div>
                  <div>
                    <div class="toggle-title">
                      自动索引
                    </div>
                    <div class="toggle-desc">
                      文件变更时自动更新索引
                    </div>
                  </div>
                </div>
                <n-switch :value="autoIndexEnabled" @update:value="toggleAutoIndex" />
              </div>

              <!-- 嵌套项目自动索引开关 -->
              <div class="auto-index-toggle mt-4">
                <div class="toggle-info">
                  <div class="toggle-icon nested-icon">
                    <div class="i-carbon-folder-parent w-5 h-5 text-amber-500" />
                  </div>
                  <div>
                    <div class="toggle-title">
                      自动索引嵌套项目
                    </div>
                    <div class="toggle-desc">
                      对父目录索引时，自动检测并索引所有 Git 子项目
                    </div>
                  </div>
                </div>
                <n-switch
                  v-model:value="config.index_nested_projects"
                  @update:value="saveConfig"
                />
              </div>

              <n-divider class="my-3" />

              <n-form-item label="防抖延迟时间" :show-feedback="false">
                <div class="debounce-input-wrapper">
                  <n-input-number
                    v-model:value="config.watch_debounce_minutes"
                    :min="1"
                    :max="30"
                    :step="1"
                    class="debounce-input"
                  />
                  <span class="debounce-unit">分钟</span>
                </div>
                <template #label>
                  <div class="form-label-with-desc">
                    <span>防抖延迟时间</span>
                    <n-tooltip trigger="hover">
                      <template #trigger>
                        <div class="i-carbon-help text-xs opacity-50 ml-1" />
                      </template>
                      文件修改后等待指定时间无新修改才触发索引更新
                    </n-tooltip>
                  </div>
                </template>
              </n-form-item>

              <div class="flex justify-end mt-3">
                <n-button type="primary" size="small" @click="saveConfig">
                  <template #icon>
                    <div class="i-carbon-save" />
                  </template>
                  保存配置
                </n-button>
              </div>
            </ConfigSection>

            <n-scrollbar class="project-list-scrollbar">
              <ProjectIndexManager />
            </n-scrollbar>
          </n-space>
        </n-scrollbar>
      </n-tab-pane>
    </n-tabs>

    <ProxySettingsModal
      v-model:show="showProxyModal"
      :config="config"
    />
  </div>
</template>

<style scoped>
.sou-config {
  height: 100%;
  display: flex;
  flex-direction: column;
}

.tab-scrollbar {
  max-height: 58vh;
}

.tab-content {
  padding-right: 12px;
  padding-bottom: 16px;
}

/* 表单反馈文字 */
.form-feedback {
  font-size: 11px;
  color: var(--color-on-surface-muted, #9ca3af);
}

/* 扩展名管理器 */
.extension-manager {
  display: flex;
  flex-direction: column;
  gap: 12px;
  width: 100%;
}

.extension-manager-head {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
}

.extension-stats,
.extension-actions {
  display: flex;
  align-items: center;
  flex-wrap: wrap;
  gap: 8px;
}

.extension-toolbar {
  display: flex;
  justify-content: flex-end;
}

.extension-presets {
  display: grid;
  grid-template-columns: repeat(2, minmax(0, 1fr));
  gap: 8px;
}

.extension-preset {
  display: flex;
  align-items: center;
  gap: 8px;
  min-width: 0;
  padding: 8px 10px;
  border-radius: 8px;
  border: 1px solid var(--color-border, rgba(128, 128, 128, 0.2));
  background: rgba(20, 184, 166, 0.04);
  color: inherit;
  text-align: left;
  cursor: pointer;
  transition: border-color 0.16s ease, background-color 0.16s ease;
}

.extension-preset:hover {
  border-color: rgba(20, 184, 166, 0.45);
  background: rgba(20, 184, 166, 0.08);
}

.extension-preset-icon {
  flex: 0 0 auto;
  font-size: 18px;
  color: rgb(20, 184, 166);
}

.extension-preset-copy {
  min-width: 0;
}

.extension-preset-title {
  font-size: 12px;
  font-weight: 600;
  line-height: 18px;
  color: var(--color-on-surface, #111827);
}

.extension-preset-desc {
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  font-size: 11px;
  line-height: 16px;
  color: var(--color-on-surface-secondary, #6b7280);
}

.extension-search {
  max-width: 220px;
}

.extension-groups {
  display: flex;
  flex-direction: column;
  gap: 10px;
}

.extension-group {
  padding-top: 10px;
  border-top: 1px solid var(--color-border, rgba(128, 128, 128, 0.18));
}

.extension-group-header {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
  margin-bottom: 8px;
}

.extension-group-title {
  display: flex;
  align-items: center;
  gap: 8px;
  min-width: 0;
}

.extension-group-actions {
  display: flex;
  align-items: center;
  gap: 6px;
}

.extension-group-icon {
  flex: 0 0 auto;
  font-size: 18px;
  color: rgb(20, 184, 166);
}

.extension-group-name {
  font-size: 13px;
  font-weight: 600;
  line-height: 18px;
  color: var(--color-on-surface, #111827);
}

.extension-group-desc {
  font-size: 11px;
  line-height: 16px;
  color: var(--color-on-surface-secondary, #6b7280);
}

.extension-chip-row,
.custom-extension-row {
  display: flex;
  flex-wrap: wrap;
  gap: 6px;
}

.custom-extension-row {
  align-items: center;
  padding-top: 10px;
  border-top: 1px dashed var(--color-border, rgba(128, 128, 128, 0.22));
}

.custom-extension-label {
  font-size: 11px;
  color: var(--color-on-surface-secondary, #6b7280);
}

.extension-chip {
  min-height: 24px;
  padding: 2px 8px;
  border-radius: 6px;
  border: 1px solid var(--color-border, rgba(128, 128, 128, 0.25));
  background: transparent;
  color: var(--color-on-surface-secondary, #4b5563);
  font-size: 12px;
  line-height: 18px;
  font-family: ui-monospace, monospace;
  cursor: pointer;
  transition: color 0.16s ease, border-color 0.16s ease, background-color 0.16s ease;
}

.extension-chip:hover {
  border-color: rgba(20, 184, 166, 0.45);
  color: var(--color-on-surface, #111827);
}

.extension-chip-selected {
  border-color: rgba(20, 184, 166, 0.55);
  background: rgba(20, 184, 166, 0.12);
  color: rgb(13, 148, 136);
}

:root.dark .extension-group {
  border-top-color: rgba(255, 255, 255, 0.08);
}

:root.dark .extension-group-name {
  color: #e5e7eb;
}

:root.dark .extension-preset {
  border-color: rgba(255, 255, 255, 0.08);
  background: rgba(20, 184, 166, 0.08);
}

:root.dark .extension-preset:hover {
  border-color: rgba(45, 212, 191, 0.45);
  background: rgba(20, 184, 166, 0.14);
}

:root.dark .extension-preset-title {
  color: #e5e7eb;
}

:root.dark .extension-group-desc,
:root.dark .extension-preset-desc,
:root.dark .custom-extension-label {
  color: #9ca3af;
}

:root.dark .custom-extension-row {
  border-top-color: rgba(255, 255, 255, 0.12);
}

:root.dark .extension-chip {
  border-color: rgba(255, 255, 255, 0.12);
  color: #d1d5db;
}

:root.dark .extension-chip:hover {
  border-color: rgba(45, 212, 191, 0.45);
  color: #f3f4f6;
}

:root.dark .extension-chip-selected {
  border-color: rgba(45, 212, 191, 0.55);
  background: rgba(20, 184, 166, 0.18);
  color: #5eead4;
}

@media (max-width: 640px) {
  .extension-manager-head,
  .extension-group-header {
    align-items: flex-start;
    flex-direction: column;
  }

  .extension-presets {
    grid-template-columns: 1fr;
  }

  .extension-toolbar {
    justify-content: stretch;
  }

  .extension-search {
    max-width: none;
  }
}

/* 信息提示 */
.info-alert {
  border-radius: 8px;
}

.compact-alert {
  border-radius: 6px;
  font-size: 12px;
}

/* 代码样式 */
.code-inline {
  padding: 2px 6px;
  border-radius: 4px;
  font-size: 12px;
  font-family: ui-monospace, monospace;
  background: var(--color-container, rgba(128, 128, 128, 0.1));
}

:root.dark .code-inline {
  background: rgba(255, 255, 255, 0.1);
}

/* 调试结果 */
.debug-result {
  margin-top: 8px;
}

.result-label {
  font-size: 12px;
  color: var(--color-on-surface-secondary, #6b7280);
  margin-bottom: 6px;
}

:root.dark .result-label {
  color: #9ca3af;
}

.result-content {
  padding: 12px;
  border-radius: 8px;
  font-size: 12px;
  font-family: ui-monospace, monospace;
  white-space: pre-wrap;
  max-height: 200px;
  overflow-y: auto;
  background: var(--color-container, rgba(128, 128, 128, 0.08));
  border: 1px solid var(--color-border, rgba(128, 128, 128, 0.2));
}

:root.dark .result-content {
  background: rgba(24, 24, 28, 0.8);
  border-color: rgba(255, 255, 255, 0.08);
}

/* 自动索引开关 */
.auto-index-toggle {
  display: flex;
  align-items: center;
  justify-content: space-between;
}

.toggle-info {
  display: flex;
  align-items: center;
  gap: 12px;
}

.toggle-icon {
  padding: 8px;
  border-radius: 8px;
  background: rgba(20, 184, 166, 0.1);
}

:root.dark .toggle-icon {
  background: rgba(20, 184, 166, 0.15);
}

/* 嵌套项目图标样式 */
.toggle-icon.nested-icon {
  background: rgba(245, 158, 11, 0.1);
}

:root.dark .toggle-icon.nested-icon {
  background: rgba(245, 158, 11, 0.15);
}

.toggle-title {
  font-size: 14px;
  font-weight: 500;
  color: var(--color-on-surface, #111827);
}

:root.dark .toggle-title {
  color: #e5e7eb;
}

.toggle-desc {
  font-size: 12px;
  color: var(--color-on-surface-secondary, #6b7280);
}

:root.dark .toggle-desc {
  color: #9ca3af;
}

/* 项目列表滚动容器 */
.project-list-scrollbar {
  max-height: 55vh;
}

/* 防抖延迟输入 */
.debounce-input-wrapper {
  display: flex;
  align-items: center;
  gap: 8px;
}

.debounce-input {
  width: 100px;
}

.debounce-unit {
  font-size: 13px;
  color: var(--color-on-surface-secondary, #6b7280);
}

:root.dark .debounce-unit {
  color: #9ca3af;
}

/* 带描述的表单标签 */
.form-label-with-desc {
  display: flex;
  align-items: center;
}

/* 调试界面 - 专业工作台布局 */
.debug-tab {
  display: flex;
  flex-direction: column;
  gap: 14px;
}

.debug-topbar {
  display: grid;
  grid-template-columns: minmax(0, 1fr) auto;
  gap: 12px;
  align-items: stretch;
  padding: 12px;
  border: 1px solid rgba(20, 184, 166, 0.16);
  border-radius: 8px;
  background:
    linear-gradient(135deg, rgba(20, 184, 166, 0.08), rgba(59, 130, 246, 0.05)),
    var(--color-container, rgba(249, 250, 251, 0.82));
  box-shadow: 0 12px 30px -24px rgba(15, 23, 42, 0.45);
}

:root.dark .debug-topbar {
  border-color: rgba(45, 212, 191, 0.16);
  background:
    linear-gradient(135deg, rgba(20, 184, 166, 0.12), rgba(59, 130, 246, 0.08)),
    rgba(24, 24, 28, 0.78);
  box-shadow: 0 16px 34px -26px rgba(0, 0, 0, 0.8);
}

.debug-topbar-main {
  min-width: 0;
}

.debug-eyebrow {
  display: inline-flex;
  align-items: center;
  gap: 6px;
  margin-bottom: 10px;
  color: rgb(13, 148, 136);
  font-size: 12px;
  font-weight: 600;
}

:root.dark .debug-eyebrow {
  color: #5eead4;
}

.debug-status-grid {
  display: grid;
  grid-template-columns: repeat(4, minmax(0, 1fr));
  gap: 8px;
}

.debug-status-pill {
  display: flex;
  align-items: center;
  min-width: 0;
  gap: 9px;
  padding: 9px 10px;
  border: 1px solid var(--debug-pill-border, rgba(148, 163, 184, 0.18));
  border-radius: 8px;
  background: var(--debug-pill-bg, rgba(255, 255, 255, 0.48));
  transition: border-color 0.18s ease, background-color 0.18s ease, transform 0.18s ease;
}

.debug-status-pill:hover {
  transform: translateY(-1px);
  border-color: var(--debug-pill-border-hover, rgba(20, 184, 166, 0.32));
}

.debug-status-pill--neutral {
  --debug-pill-bg: rgba(148, 163, 184, 0.08);
  --debug-pill-border: rgba(148, 163, 184, 0.18);
  --debug-pill-color: #64748b;
}

.debug-status-pill--info {
  --debug-pill-bg: rgba(59, 130, 246, 0.08);
  --debug-pill-border: rgba(59, 130, 246, 0.18);
  --debug-pill-color: #3b82f6;
}

.debug-status-pill--success {
  --debug-pill-bg: rgba(20, 184, 166, 0.09);
  --debug-pill-border: rgba(20, 184, 166, 0.22);
  --debug-pill-color: #0d9488;
}

.debug-status-pill--warning {
  --debug-pill-bg: rgba(245, 158, 11, 0.08);
  --debug-pill-border: rgba(245, 158, 11, 0.2);
  --debug-pill-color: #d97706;
}

.debug-status-pill--danger {
  --debug-pill-bg: rgba(244, 63, 94, 0.08);
  --debug-pill-border: rgba(244, 63, 94, 0.2);
  --debug-pill-color: #e11d48;
}

:root.dark .debug-status-pill {
  background: var(--debug-pill-bg, rgba(255, 255, 255, 0.05));
  border-color: var(--debug-pill-border, rgba(255, 255, 255, 0.08));
}

.debug-status-icon {
  flex: 0 0 auto;
  display: grid;
  place-items: center;
  width: 30px;
  height: 30px;
  border-radius: 8px;
  color: var(--debug-pill-color, #64748b);
  background: rgba(255, 255, 255, 0.55);
  font-size: 16px;
}

:root.dark .debug-status-icon {
  background: rgba(255, 255, 255, 0.07);
}

.debug-status-copy {
  min-width: 0;
}

.debug-status-label,
.debug-status-detail {
  overflow: hidden;
  color: var(--color-on-surface-muted, #9ca3af);
  text-overflow: ellipsis;
  white-space: nowrap;
  font-size: 11px;
  line-height: 15px;
}

.debug-status-value {
  overflow: hidden;
  margin: 1px 0;
  color: var(--color-on-surface, #111827);
  text-overflow: ellipsis;
  white-space: nowrap;
  font-size: 13px;
  font-weight: 650;
  line-height: 18px;
}

:root.dark .debug-status-value {
  color: #f3f4f6;
}

.debug-topbar-actions {
  display: flex;
  flex-direction: column;
  justify-content: center;
  gap: 8px;
  min-width: 104px;
}

.debug-note {
  margin-top: -2px;
}

.debug-console {
  display: flex;
  flex-direction: column;
  gap: 12px;
}

.debug-console-row {
  display: grid;
  gap: 12px;
}

.debug-console-row--controls {
  grid-template-columns: minmax(0, 1fr) minmax(190px, 0.38fr);
}

.debug-console-row--query {
  grid-template-columns: minmax(0, 1fr) 150px;
  align-items: stretch;
}

.debug-field {
  margin-bottom: 0;
}

.debug-field-label {
  display: inline-flex;
  align-items: center;
  gap: 8px;
}

.debug-run-panel {
  display: flex;
  flex-direction: column;
  justify-content: flex-end;
  gap: 8px;
  padding-top: 22px;
}

.debug-run-button {
  width: 100%;
  min-height: 40px;
}

.debug-run-hint {
  color: var(--color-on-surface-muted, #9ca3af);
  font-size: 11px;
  line-height: 16px;
}

/* 调试界面 - 骨架屏 */
.debug-skeleton {
  margin-top: 12px;
  padding: 14px;
  border: 1px solid rgba(20, 184, 166, 0.16);
  border-radius: 8px;
  background: rgba(20, 184, 166, 0.05);
}

:root.dark .debug-skeleton {
  border-color: rgba(45, 212, 191, 0.14);
  background: rgba(20, 184, 166, 0.08);
}

.debug-skeleton-header {
  display: inline-flex;
  align-items: center;
  gap: 8px;
  margin-bottom: 10px;
  color: rgb(13, 148, 136);
  font-size: 12px;
  font-weight: 600;
}

:root.dark .debug-skeleton-header {
  color: #5eead4;
}

/* 调试界面 - 结果工作台 */
.debug-result-panel {
  margin-top: 12px;
  overflow: hidden;
  border: 1px solid var(--color-border, rgba(128, 128, 128, 0.14));
  border-radius: 8px;
  background: rgba(255, 255, 255, 0.72);
  box-shadow: 0 14px 28px -24px rgba(15, 23, 42, 0.55);
}

:root.dark .debug-result-panel {
  border-color: rgba(255, 255, 255, 0.08);
  background: rgba(20, 20, 25, 0.62);
  box-shadow: 0 16px 34px -28px rgba(0, 0, 0, 0.9);
}

.debug-result-header {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
  padding: 14px 16px;
  border-bottom: 1px solid var(--color-border, rgba(128, 128, 128, 0.14));
  background: linear-gradient(135deg, rgba(20, 184, 166, 0.08), rgba(148, 163, 184, 0.05));
}

.debug-result-header.is-error {
  background: linear-gradient(135deg, rgba(244, 63, 94, 0.08), rgba(245, 158, 11, 0.05));
}

:root.dark .debug-result-header {
  border-bottom-color: rgba(255, 255, 255, 0.08);
  background: linear-gradient(135deg, rgba(20, 184, 166, 0.12), rgba(148, 163, 184, 0.06));
}

:root.dark .debug-result-header.is-error {
  background: linear-gradient(135deg, rgba(244, 63, 94, 0.12), rgba(245, 158, 11, 0.07));
}

.debug-result-title-group,
.debug-result-actions {
  display: flex;
  align-items: center;
  gap: 10px;
  min-width: 0;
}

.debug-result-actions {
  flex: 0 0 auto;
}

.debug-result-icon {
  display: grid;
  flex: 0 0 auto;
  place-items: center;
  width: 34px;
  height: 34px;
  border-radius: 8px;
  color: #0d9488;
  background: rgba(20, 184, 166, 0.12);
  font-size: 18px;
}

.debug-result-header.is-error .debug-result-icon {
  color: #e11d48;
  background: rgba(244, 63, 94, 0.12);
}

.debug-result-title {
  color: var(--color-on-surface, #111827);
  font-size: 14px;
  font-weight: 650;
  line-height: 20px;
}

.debug-result-subtitle {
  overflow: hidden;
  max-width: 430px;
  color: var(--color-on-surface-secondary, #6b7280);
  text-overflow: ellipsis;
  white-space: nowrap;
  font-size: 12px;
  line-height: 17px;
}

:root.dark .debug-result-title {
  color: #f3f4f6;
}

:root.dark .debug-result-subtitle {
  color: #9ca3af;
}

.result-section {
  padding: 13px 16px;
  border-bottom: 1px solid var(--color-border, rgba(128, 128, 128, 0.1));
}

.result-section:last-child {
  border-bottom: none;
}

:root.dark .result-section {
  border-bottom-color: rgba(255, 255, 255, 0.06);
}

.result-section--metrics {
  display: grid;
  grid-template-columns: repeat(3, minmax(0, 1fr));
  gap: 10px;
  background: rgba(148, 163, 184, 0.05);
}

:root.dark .result-section--metrics {
  background: rgba(255, 255, 255, 0.03);
}

.section-header {
  display: flex;
  align-items: center;
  gap: 8px;
  margin-bottom: 10px;
  color: var(--color-on-surface, #374151);
  font-size: 13px;
  font-weight: 600;
}

:root.dark .section-header {
  color: #e5e7eb;
}

.section-content {
  font-size: 12px;
}

.debug-info-grid {
  display: grid;
  gap: 8px;
}

.info-row {
  display: grid;
  grid-template-columns: 72px minmax(0, 1fr);
  align-items: flex-start;
  gap: 10px;
}

.info-label {
  color: var(--color-on-surface-muted, #9ca3af);
  white-space: nowrap;
}

.info-value {
  min-width: 0;
  color: var(--color-on-surface, #374151);
  word-break: break-word;
}

:root.dark .info-value {
  color: #d1d5db;
}

/* 调试界面 - 性能指标 */
.metric-item {
  min-width: 0;
  padding: 10px 12px;
  border: 1px solid var(--color-border, rgba(128, 128, 128, 0.12));
  border-radius: 8px;
  background: rgba(255, 255, 255, 0.62);
}

:root.dark .metric-item {
  border-color: rgba(255, 255, 255, 0.08);
  background: rgba(255, 255, 255, 0.045);
}

.metric-value {
  overflow: hidden;
  margin-bottom: 3px;
  color: var(--color-on-surface, #111827);
  text-overflow: ellipsis;
  white-space: nowrap;
  font-size: 18px;
  font-weight: 700;
  line-height: 24px;
}

.metric-value--small {
  font-size: 13px;
  font-weight: 650;
}

:root.dark .metric-value {
  color: #f3f4f6;
}

.metric-label {
  color: var(--color-on-surface-muted, #9ca3af);
  font-size: 11px;
  line-height: 15px;
}

/* 调试界面 - 错误内容 */
.error-content {
  padding: 12px;
  border: 1px solid rgba(244, 63, 94, 0.18);
  border-radius: 8px;
  background: rgba(244, 63, 94, 0.08);
  color: #be123c;
  font-size: 12px;
  line-height: 1.55;
}

:root.dark .error-content {
  border-color: rgba(251, 113, 133, 0.22);
  background: rgba(244, 63, 94, 0.14);
  color: #fda4af;
}

/* 调试界面 - 结果预览 */
.result-scrollbar {
  max-height: 260px;
}

.result-pre {
  margin: 0;
  padding: 13px;
  border: 1px solid rgba(148, 163, 184, 0.16);
  border-radius: 8px;
  background: rgba(15, 23, 42, 0.035);
  color: var(--color-on-surface, #374151);
  white-space: pre-wrap;
  word-break: break-word;
  font-size: 12px;
  font-family: ui-monospace, monospace;
  line-height: 1.55;
}

:root.dark .result-pre {
  border-color: rgba(255, 255, 255, 0.08);
  background: rgba(0, 0, 0, 0.28);
  color: #d1d5db;
}

@media (max-width: 820px) {
  .debug-topbar,
  .debug-console-row--query {
    grid-template-columns: 1fr;
  }

  .debug-topbar-actions {
    flex-direction: row;
    justify-content: flex-start;
  }

  .debug-status-grid,
  .debug-console-row--controls {
    grid-template-columns: repeat(2, minmax(0, 1fr));
  }

  .debug-run-panel {
    padding-top: 0;
  }
}

@media (max-width: 560px) {
  .debug-status-grid,
  .debug-console-row--controls,
  .result-section--metrics {
    grid-template-columns: 1fr;
  }

  .debug-result-header {
    align-items: flex-start;
    flex-direction: column;
  }

  .debug-result-actions {
    flex-wrap: wrap;
  }

  .info-row {
    grid-template-columns: 1fr;
    gap: 4px;
  }
}
</style>
