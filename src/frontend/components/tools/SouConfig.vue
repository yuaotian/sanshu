<script setup lang="ts">
/**
 * 代码搜索工具 (Acemcp/Sou) 配置组件
 * 包含：基础配置、高级配置、日志调试、索引管理
 */
import { invoke } from '@tauri-apps/api/core'
import { useMessage } from 'naive-ui'
import { onMounted, ref, watch } from 'vue'
import { useAcemcpSync } from '../../composables/useAcemcpSync'
import { useLogViewer } from '../../composables/useLogViewer'
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
})

const loadingConfig = ref(false)
const showProxyModal = ref(false)
const logFilePath = ref('')
const lastSavedConnection = ref({
  base_url: '',
  token: '',
})
const { open: openLogViewer } = useLogViewer()
// 调试状态
const debugProjectRoot = ref('')
const debugQuery = ref('')
const debugLoading = ref(false)
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

const debugResultData = ref<DebugSearchResult | null>(null)

// 选项数据
const extOptions = ref([
  '.py',
  '.js',
  '.ts',
  '.jsx',
  '.tsx',
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
  '.md',
  '.txt',
  '.json',
  '.yaml',
  '.yml',
  '.toml',
  '.xml',
  '.html',
  '.css',
  '.scss',
  '.sql',
  '.sh',
  '.bash',
].map(v => ({ label: v, value: v })))

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

async function loadLogFilePath() {
  try {
    const path = await invoke('get_acemcp_log_file_path') as string
    logFilePath.value = path || ''
  }
  catch {
    logFilePath.value = ''
  }
}

async function saveConfig() {
  try {
    if (!config.value.base_url || !/^https?:\/\//i.test(config.value.base_url)) {
      message.error('URL无效，需以 http(s):// 开头')
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
    } as Intl.DateTimeFormatOptions)
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

async function viewLogs() {
  try {
    const lines = await invoke('read_acemcp_logs') as string[]
    if (lines.length > 0) {
      await navigator.clipboard.writeText(lines.join('\n'))
      message.success(`已复制 ${lines.length} 行日志`)
    }
    else {
      message.info('日志为空')
    }
  }
  catch (e) {
    message.error(`读取日志失败: ${e}`)
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

// 监听扩展名变化，自动规范化
watch(() => config.value.text_extensions, (list) => {
  const norm = Array.from(new Set((list || []).map((s) => {
    const t = s.trim().toLowerCase()
    return t ? (t.startsWith('.') ? t : `.${t}`) : ''
  }).filter(Boolean)))

  if (norm.join(',') !== list.join(',')) {
    config.value.text_extensions = norm
  }
}, { deep: true })

// 组件挂载
onMounted(async () => {
  if (props.active) {
    await loadAcemcpConfig()
    await loadLogFilePath()
    await Promise.all([
      fetchAutoIndexEnabled(),
      fetchWatchingProjects(),
    ])
  }
})

defineExpose({ saveConfig })
</script>

<template>
  <div class="h-full flex flex-col">
    <n-tabs type="line" animated>
      <!-- 基础配置 -->
      <n-tab-pane name="basic" tab="基础配置">
        <n-scrollbar class="max-h-[58vh]">
          <n-space vertical size="medium" class="pr-3 pb-4">
            <ConfigSection title="连接设置" description="配置代码搜索服务的连接信息">
              <n-grid :x-gap="24" :y-gap="16" :cols="1">
                <n-grid-item>
                  <div>
                    <div class="text-xs text-on-surface-secondary mb-1">
                      API端点URL
                    </div>
                    <n-input v-model:value="config.base_url" size="small" placeholder="https://api.example.com" clearable />
                  </div>
                </n-grid-item>
                <n-grid-item>
                  <div>
                    <div class="text-xs text-on-surface-secondary mb-1">
                      认证令牌
                    </div>
                    <n-input
                      v-model:value="config.token"
                      size="small"
                      type="password"
                      show-password-on="click"
                      placeholder="输入认证令牌"
                      clearable
                    />
                  </div>
                </n-grid-item>
              </n-grid>
            </ConfigSection>

            <ConfigSection title="性能参数" description="调整处理批量和文件大小限制">
              <n-grid :x-gap="24" :cols="2">
                <n-grid-item>
                  <div>
                    <div class="text-xs text-on-surface-secondary mb-1">
                      批处理大小
                    </div>
                    <n-input-number v-model:value="config.batch_size" size="small" :min="1" :max="100" class="w-full" />
                  </div>
                </n-grid-item>
                <n-grid-item>
                  <div>
                    <div class="text-xs text-on-surface-secondary mb-1">
                      最大行数/块
                    </div>
                    <n-input-number v-model:value="config.max_lines_per_blob" size="small" :min="100" :max="5000" class="w-full" />
                  </div>
                </n-grid-item>
              </n-grid>
            </ConfigSection>

            <ConfigSection title="代理设置" description="配置 HTTP/HTTPS 代理以优化网络连接及访问速度">
              <n-card size="small">
                <div class="flex items-center justify-between">
                  <div class="flex items-center gap-3">
                    <div class="i-carbon-network-3 text-lg" :class="config.proxy_enabled ? 'text-success' : 'text-on-surface-muted'" />
                    <div>
                      <div class="text-sm font-medium flex items-center gap-2">
                        代理服务
                        <n-tag :type="config.proxy_enabled ? 'success' : 'default'" size="small" :bordered="false">
                          {{ config.proxy_enabled ? '已启用' : '未启用' }}
                        </n-tag>
                      </div>
                      <div class="text-xs text-on-surface-muted mt-0.5">
                        <template v-if="config.proxy_enabled">
                          {{ config.proxy_type.toUpperCase() }}://{{ config.proxy_host }}:{{ config.proxy_port }}
                        </template>
                        <template v-else>
                          当前使用直连模式，配置代理可加速海外 API 访问
                        </template>
                      </div>
                    </div>
                  </div>
                  <n-button size="small" secondary @click="showProxyModal = true">
                    <template #icon>
                      <div class="i-carbon-settings-adjust" />
                    </template>
                    配置代理与诊断
                  </n-button>
                </div>
              </n-card>
            </ConfigSection>

            <div class="flex justify-end">
              <n-button type="primary" size="small" @click="saveConfig">
                <template #icon>
                  <div class="i-carbon-save" />
                </template>
                保存配置
              </n-button>
            </div>
          </n-space>
        </n-scrollbar>
      </n-tab-pane>

      <!-- 高级配置 -->
      <n-tab-pane name="advanced" tab="高级配置">
        <n-scrollbar class="max-h-[58vh]">
          <n-space vertical size="medium" class="pr-3 pb-4">
            <ConfigSection title="文件过滤" description="设置需索引的文件类型和排除规则">
              <n-space vertical size="medium">
                <div>
                  <div class="text-xs text-on-surface-secondary mb-1">
                    包含扩展名
                  </div>
                  <n-select
                    v-model:value="config.text_extensions"
                    size="small"
                    :options="extOptions"
                    multiple tag filterable clearable
                    placeholder="输入或选择扩展名 (.py)"
                  />
                  <div class="text-xs text-on-surface-muted mt-1">
                    小写，点开头，自动去重
                  </div>
                </div>

                <div>
                  <div class="text-xs text-on-surface-secondary mb-1">
                    排除模式
                  </div>
                  <n-select
                    v-model:value="config.exclude_patterns"
                    size="small"
                    :options="excludeOptions"
                    multiple tag filterable clearable
                    placeholder="输入或选择排除模式 (node_modules)"
                  />
                  <div class="text-xs text-on-surface-muted mt-1">
                    支持 glob 通配符
                  </div>
                </div>
              </n-space>
            </ConfigSection>

            <div class="flex justify-end">
              <n-button type="primary" size="small" @click="saveConfig">
                <template #icon>
                  <div class="i-carbon-save" />
                </template>
                保存配置
              </n-button>
            </div>
          </n-space>
        </n-scrollbar>
      </n-tab-pane>

      <!-- 日志与调试 -->
      <n-tab-pane name="debug" tab="日志与调试">
        <n-scrollbar class="max-h-[58vh]">
          <n-space vertical size="medium" class="pr-3 pb-4">
            <ConfigSection title="工具状态">
              <n-alert type="info" :bordered="false">
                <template #icon>
                  <div class="i-carbon-terminal" />
                </template>
                日志路径: <n-text code>{{ logFilePath || '默认路径' }}</n-text>
              </n-alert>
              <n-space class="mt-2">
                <n-button size="small" secondary @click="testConnection">
                  <template #icon>
                    <div class="i-carbon-connection-signal" />
                  </template>
                  测试连接
                </n-button>
                <n-button size="small" secondary @click="viewLogs">
                  <template #icon>
                    <div class="i-carbon-document" />
                  </template>
                  查看日志
                </n-button>
                <n-button size="small" secondary @click="openLogViewer()">
                  <template #icon>
                    <div class="i-carbon-view" />
                  </template>
                  实时日志
                </n-button>
                <n-button size="small" secondary @click="clearCache">
                  <template #icon>
                    <div class="i-carbon-clean" />
                  </template>
                  清除缓存
                </n-button>
              </n-space>
            </ConfigSection>

            <ConfigSection title="运行状态">
              <n-descriptions :column="3" label-placement="left" size="small" bordered>
                <n-descriptions-item label="代理状态">
                  <n-tag :type="config.proxy_enabled ? 'success' : 'default'" size="small">
                    {{ config.proxy_enabled ? '已启用' : '未启用' }}
                  </n-tag>
                </n-descriptions-item>
                <n-descriptions-item label="索引项目">
                  {{ debugProjectOptions.length }} 个
                </n-descriptions-item>
                <n-descriptions-item label="上次调试">
                  <n-tag v-if="debugResultData" :type="debugResultData.success ? 'success' : 'error'" size="small">
                    {{ debugResultData.success ? '成功' : '失败' }} ({{ debugResultData.total_duration_ms }}ms)
                  </n-tag>
                  <n-text v-else depth="3">
                    未执行
                  </n-text>
                </n-descriptions-item>
              </n-descriptions>
            </ConfigSection>

            <ConfigSection title="搜索调试" description="模拟搜索请求以验证配置">
              <n-space vertical size="medium">
                <div>
                  <div class="text-xs text-on-surface-secondary mb-1 flex items-center gap-2">
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
                  <n-select
                    v-if="!debugUseManualInput"
                    v-model:value="debugProjectRoot"
                    size="small"
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
                    size="small"
                    placeholder="/abs/path/to/project"
                    clearable
                  />
                </div>

                <div>
                  <div class="text-xs text-on-surface-secondary mb-1">
                    查询语句
                  </div>
                  <n-input
                    v-model:value="debugQuery"
                    size="small"
                    type="textarea"
                    :rows="2"
                    placeholder="输入搜索意图..."
                  />
                </div>

                <n-button
                  type="primary"
                  ghost
                  size="small"
                  :loading="debugLoading"
                  :disabled="!debugProjectRoot || !debugQuery"
                  @click="runToolDebug"
                >
                  <template #icon>
                    <div class="i-carbon-play" />
                  </template>
                  运行调试
                </n-button>

                <n-spin v-if="debugLoading" size="small" />

                <n-collapse-transition :show="debugResultData !== null && !debugLoading">
                  <n-card v-if="debugResultData" size="small">
                    <n-descriptions :column="2" label-placement="left" size="small" bordered>
                      <n-descriptions-item label="项目">
                        <n-text code>{{ debugResultData.project_path }}</n-text>
                      </n-descriptions-item>
                      <n-descriptions-item label="查询">
                        {{ debugResultData.query }}
                      </n-descriptions-item>
                      <n-descriptions-item label="发送时间">
                        {{ formatDebugTime(debugResultData.request_time) }}
                      </n-descriptions-item>
                      <n-descriptions-item label="耗时">
                        <n-text :type="debugResultData.success ? 'success' : 'error'">
                          {{ debugResultData.total_duration_ms }}ms
                        </n-text>
                      </n-descriptions-item>
                      <n-descriptions-item label="结果数">
                        {{ debugResultData.result_count ?? '-' }}
                      </n-descriptions-item>
                      <n-descriptions-item label="状态">
                        <n-tag :type="debugResultData.success ? 'success' : 'error'" size="small">
                          {{ debugResultData.success ? '成功' : '失败' }}
                        </n-tag>
                      </n-descriptions-item>
                    </n-descriptions>

                    <n-alert
                      v-if="debugResultData.error"
                      type="error"
                      :bordered="false"
                      class="mt-2"
                    >
                      {{ debugResultData.error }}
                    </n-alert>

                    <template v-if="debugResultData.success && debugResultData.result">
                      <div class="flex items-center justify-between mt-2 mb-1">
                        <n-text depth="3" class="text-xs">
                          响应数据
                        </n-text>
                        <n-button size="tiny" text @click="copyDebugResult">
                          <template #icon>
                            <div class="i-carbon-copy" />
                          </template>
                          复制
                        </n-button>
                      </div>
                      <n-scrollbar style="max-height: 200px">
                        <n-code :code="debugResultData.result" language="text" word-wrap />
                      </n-scrollbar>
                    </template>
                  </n-card>
                </n-collapse-transition>
              </n-space>
            </ConfigSection>
          </n-space>
        </n-scrollbar>
      </n-tab-pane>

      <!-- 索引管理 -->
      <n-tab-pane name="index" tab="索引管理">
        <n-scrollbar class="max-h-[58vh]">
          <n-space vertical size="medium" class="pr-3 pb-4">
            <ConfigSection title="全局策略">
              <n-space vertical size="small">
                <n-card size="small">
                  <div class="flex items-center justify-between">
                    <div>
                      <div class="text-sm font-medium">
                        自动索引
                      </div>
                      <n-text depth="3" class="text-xs">
                        文件变更时自动更新索引
                      </n-text>
                    </div>
                    <n-switch :value="autoIndexEnabled" size="small" @update:value="toggleAutoIndex" />
                  </div>
                </n-card>

                <n-card size="small">
                  <div class="flex items-center justify-between">
                    <div>
                      <div class="text-sm font-medium">
                        自动索引嵌套项目
                      </div>
                      <n-text depth="3" class="text-xs">
                        对父目录索引时，自动检测并索引所有 Git 子项目
                      </n-text>
                    </div>
                    <n-switch
                      v-model:value="config.index_nested_projects"
                      size="small"
                      @update:value="saveConfig"
                    />
                  </div>
                </n-card>

                <div>
                  <div class="text-xs text-on-surface-secondary mb-1 flex items-center">
                    <span>防抖延迟时间</span>
                    <n-tooltip trigger="hover">
                      <template #trigger>
                        <div class="i-carbon-help text-xs opacity-50 ml-1" />
                      </template>
                      文件修改后等待指定时间无新修改才触发索引更新
                    </n-tooltip>
                  </div>
                  <div class="flex items-center gap-2">
                    <n-input-number
                      v-model:value="config.watch_debounce_minutes"
                      size="small"
                      :min="1"
                      :max="30"
                      :step="1"
                      class="w-[100px]"
                    />
                    <n-text depth="3" class="text-xs">
                      分钟
                    </n-text>
                  </div>
                </div>

                <div class="flex justify-end">
                  <n-button type="primary" size="small" @click="saveConfig">
                    <template #icon>
                      <div class="i-carbon-save" />
                    </template>
                    保存配置
                  </n-button>
                </div>
              </n-space>
            </ConfigSection>

            <n-scrollbar class="max-h-[55vh]">
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
