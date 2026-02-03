<script setup lang="ts">
/**
 * 代码搜索工具 (Acemcp/Sou) 配置组件
 * 包含：基础配置、高级配置、日志调试、索引管理
 */
import { invoke } from '@tauri-apps/api/core'
import { useMessage } from 'naive-ui'
import { onMounted, ref, watch } from 'vue'
import { useAcemcpSync } from '../../composables/useAcemcpSync'
import ConfigSection from '../common/ConfigSection.vue'
import ProjectIndexManager from '../settings/ProjectIndexManager.vue'
import AcemcpLogViewerDrawer from './AcemcpLogViewerDrawer.vue'
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
const showLogViewer = ref(false)
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
    message.success('配置已保存')
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
            <ConfigSection title="连接设置" description="配置代码搜索服务的连接信息">
              <n-grid :x-gap="24" :y-gap="16" :cols="1">
                <n-grid-item>
                  <n-form-item label="API端点URL">
                    <n-input v-model:value="config.base_url" placeholder="https://api.example.com" clearable />
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
                    />
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

      <!-- 高级配置 -->
      <n-tab-pane name="advanced" tab="高级配置">
        <n-scrollbar class="tab-scrollbar">
          <n-space vertical size="large" class="tab-content">
            <ConfigSection title="文件过滤" description="设置需索引的文件类型和排除规则">
              <n-space vertical size="medium">
                <n-form-item label="包含扩展名">
                  <n-select
                    v-model:value="config.text_extensions"
                    :options="extOptions"
                    multiple tag filterable clearable
                    placeholder="输入或选择扩展名 (.py)"
                  />
                  <template #feedback>
                    <span class="form-feedback">小写，点开头，自动去重</span>
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

      <!-- 日志与调试 -->
      <n-tab-pane name="debug" tab="日志与调试">
        <n-scrollbar class="tab-scrollbar">
          <n-space vertical size="large" class="tab-content">
            <ConfigSection title="工具状态" :no-card="true">
              <n-alert type="info" :bordered="false" class="info-alert">
                <template #icon>
                  <div class="i-carbon-terminal" />
                </template>
                日志路径: <code class="code-inline">%APPDATA%/sanshu/log/acemcp.log</code>（Windows）
              </n-alert>

              <n-space class="mt-3">
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
                <n-button size="small" secondary @click="showLogViewer = true">
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

            <!-- 状态信息卡片 -->
            <ConfigSection title="运行状态" :no-card="true">
              <n-grid :x-gap="12" :y-gap="12" :cols="12">
                <n-grid-item :span="4">
                  <div class="status-card">
                    <div class="status-icon">
                      <div :class="config.proxy_enabled ? 'i-carbon-checkmark-outline text-emerald-500' : 'i-carbon-close-outline text-slate-400'" />
                    </div>
                    <div class="status-info">
                      <div class="status-title">
                        代理状态
                      </div>
                      <div class="status-value">
                        {{ config.proxy_enabled ? '已启用' : '未启用' }}
                      </div>
                      <div v-if="config.proxy_enabled" class="status-detail">
                        {{ config.proxy_host }}:{{ config.proxy_port }}
                      </div>
                    </div>
                  </div>
                </n-grid-item>
                <n-grid-item :span="4">
                  <div class="status-card">
                    <div class="status-icon">
                      <div class="i-carbon-folder-shared text-blue-500" />
                    </div>
                    <div class="status-info">
                      <div class="status-title">
                        索引项目
                      </div>
                      <div class="status-value">
                        {{ debugProjectOptions.length }} 个
                      </div>
                    </div>
                  </div>
                </n-grid-item>
                <n-grid-item :span="4">
                  <div class="status-card">
                    <div class="status-icon">
                      <div :class="debugResultData?.success ? 'i-carbon-checkmark-filled text-emerald-500' : (debugResultData === null ? 'i-carbon-pending text-slate-400' : 'i-carbon-warning-alt text-amber-500')" />
                    </div>
                    <div class="status-info">
                      <div class="status-title">
                        上次调试
                      </div>
                      <div class="status-value">
                        {{ debugResultData ? (debugResultData.success ? '成功' : '失败') : '未执行' }}
                      </div>
                      <div v-if="debugResultData" class="status-detail">
                        {{ debugResultData.total_duration_ms }}ms
                      </div>
                    </div>
                  </div>
                </n-grid-item>
              </n-grid>
            </ConfigSection>

            <ConfigSection title="搜索调试" description="模拟搜索请求以验证配置">
              <n-space vertical size="medium">
                <!-- 项目选择 -->
                <n-form-item :show-feedback="false">
                  <template #label>
                    <div class="flex items-center gap-2">
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

                <n-form-item label="查询语句" :show-feedback="false">
                  <n-input
                    v-model:value="debugQuery"
                    type="textarea"
                    :rows="2"
                    placeholder="输入搜索意图..."
                  />
                </n-form-item>

                <n-button
                  type="primary"
                  ghost
                  :loading="debugLoading"
                  :disabled="!debugProjectRoot || !debugQuery"
                  @click="runToolDebug"
                >
                  <template #icon>
                    <div class="i-carbon-play" />
                  </template>
                  运行调试
                </n-button>

                <!-- 骨架屏加载态 -->
                <div v-if="debugLoading" class="debug-skeleton">
                  <n-skeleton text :repeat="3" />
                  <n-skeleton text style="width: 60%" />
                </div>

                <!-- 结构化结果展示 -->
                <n-collapse-transition :show="debugResultData !== null && !debugLoading">
                  <div v-if="debugResultData" class="debug-result-panel">
                    <!-- 请求信息 -->
                    <div class="result-section">
                      <div class="section-header">
                        <div class="i-carbon-send text-blue-500" />
                        <span>请求信息</span>
                      </div>
                      <div class="section-content">
                        <div class="info-row">
                          <span class="info-label">项目:</span>
                          <code class="info-value">{{ debugResultData.project_path }}</code>
                        </div>
                        <div class="info-row">
                          <span class="info-label">查询:</span>
                          <span class="info-value">{{ debugResultData.query }}</span>
                        </div>
                        <div class="info-row">
                          <span class="info-label">发送时间:</span>
                          <span class="info-value">{{ formatDebugTime(debugResultData.request_time) }}</span>
                        </div>
                      </div>
                    </div>

                    <!-- 性能指标 -->
                    <div class="result-section">
                      <div class="section-header">
                        <div class="i-carbon-timer text-amber-500" />
                        <span>性能指标</span>
                      </div>
                      <div class="section-content">
                        <n-grid :x-gap="12" :cols="12">
                          <n-grid-item :span="4">
                            <div class="metric-item">
                              <div class="metric-value" :class="debugResultData.success ? 'text-emerald-500' : 'text-red-500'">
                                {{ debugResultData.total_duration_ms }}ms
                              </div>
                              <div class="metric-label">
                                总耗时
                              </div>
                            </div>
                          </n-grid-item>
                          <n-grid-item :span="4">
                            <div class="metric-item">
                              <div class="metric-value">
                                {{ debugResultData.result_count ?? '-' }}
                              </div>
                              <div class="metric-label">
                                结果数
                              </div>
                            </div>
                          </n-grid-item>
                          <n-grid-item :span="4">
                            <div class="metric-item">
                              <n-tag :type="debugResultData.success ? 'success' : 'error'" size="small">
                                {{ debugResultData.success ? '成功' : '失败' }}
                              </n-tag>
                              <div class="metric-label">
                                状态
                              </div>
                            </div>
                          </n-grid-item>
                        </n-grid>
                      </div>
                    </div>

                    <!-- 响应数据 / 错误信息 -->
                    <div class="result-section">
                      <div class="section-header">
                        <div :class="debugResultData.success ? 'i-carbon-document text-emerald-500' : 'i-carbon-warning text-red-500'" />
                        <span>{{ debugResultData.success ? '响应数据' : '错误信息' }}</span>
                        <n-button
                          v-if="debugResultData.success && debugResultData.result"
                          size="tiny"
                          text
                          class="ml-auto"
                          @click="copyDebugResult"
                        >
                          <template #icon>
                            <div class="i-carbon-copy" />
                          </template>
                          复制
                        </n-button>
                      </div>
                      <div class="section-content">
                        <div v-if="debugResultData.error" class="error-content">
                          {{ debugResultData.error }}
                        </div>
                        <n-scrollbar v-else style="max-height: 200px">
                          <pre class="result-pre">{{ debugResultData.result || '无返回结果' }}</pre>
                        </n-scrollbar>
                      </div>
                    </div>
                  </div>
                </n-collapse-transition>
              </n-space>
            </ConfigSection>
          </n-space>
        </n-scrollbar>
      </n-tab-pane>

      <!-- 索引管理 -->
      <n-tab-pane name="index" tab="索引管理">
        <n-scrollbar class="tab-scrollbar">
          <n-space vertical size="large" class="tab-content">
            <ConfigSection title="全局策略">
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

    <AcemcpLogViewerDrawer v-model:show="showLogViewer" />
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

/* 信息提示 */
.info-alert {
  border-radius: 8px;
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

/* 调试界面 - 状态卡片 */
.status-card {
  display: flex;
  align-items: center;
  gap: 12px;
  padding: 12px;
  border-radius: 10px;
  background: var(--color-container, rgba(128, 128, 128, 0.06));
  border: 1px solid var(--color-border, rgba(128, 128, 128, 0.12));
  transition: all 0.2s ease;
}

.status-card:hover {
  border-color: rgba(99, 102, 241, 0.3);
}

:root.dark .status-card {
  background: rgba(30, 30, 35, 0.6);
  border-color: rgba(255, 255, 255, 0.08);
}

.status-icon {
  font-size: 20px;
}

.status-info {
  flex: 1;
  min-width: 0;
}

.status-title {
  font-size: 11px;
  color: var(--color-on-surface-muted, #9ca3af);
  margin-bottom: 2px;
}

.status-value {
  font-size: 14px;
  font-weight: 600;
  color: var(--color-on-surface, #111827);
}

:root.dark .status-value {
  color: #f3f4f6;
}

.status-detail {
  font-size: 11px;
  color: var(--color-on-surface-muted, #9ca3af);
  margin-top: 2px;
  font-family: ui-monospace, monospace;
}

/* 调试界面 - 骨架屏 */
.debug-skeleton {
  padding: 16px;
  border-radius: 10px;
  background: var(--color-container, rgba(128, 128, 128, 0.06));
  border: 1px solid var(--color-border, rgba(128, 128, 128, 0.12));
}

:root.dark .debug-skeleton {
  background: rgba(30, 30, 35, 0.6);
  border-color: rgba(255, 255, 255, 0.08);
}

/* 调试界面 - 结果面板 */
.debug-result-panel {
  border-radius: 10px;
  background: var(--color-container, rgba(128, 128, 128, 0.04));
  border: 1px solid var(--color-border, rgba(128, 128, 128, 0.12));
  overflow: hidden;
}

:root.dark .debug-result-panel {
  background: rgba(20, 20, 25, 0.5);
  border-color: rgba(255, 255, 255, 0.08);
}

.result-section {
  padding: 12px 16px;
  border-bottom: 1px solid var(--color-border, rgba(128, 128, 128, 0.1));
}

.result-section:last-child {
  border-bottom: none;
}

:root.dark .result-section {
  border-bottom-color: rgba(255, 255, 255, 0.06);
}

.section-header {
  display: flex;
  align-items: center;
  gap: 8px;
  font-size: 13px;
  font-weight: 500;
  color: var(--color-on-surface, #374151);
  margin-bottom: 10px;
}

:root.dark .section-header {
  color: #e5e7eb;
}

.section-content {
  font-size: 12px;
}

.info-row {
  display: flex;
  align-items: flex-start;
  gap: 8px;
  margin-bottom: 6px;
}

.info-row:last-child {
  margin-bottom: 0;
}

.info-label {
  color: var(--color-on-surface-muted, #9ca3af);
  white-space: nowrap;
  min-width: 60px;
}

.info-value {
  color: var(--color-on-surface, #374151);
  word-break: break-all;
}

:root.dark .info-value {
  color: #d1d5db;
}

/* 调试界面 - 性能指标 */
.metric-item {
  text-align: center;
  padding: 8px;
  border-radius: 8px;
  background: rgba(128, 128, 128, 0.04);
}

:root.dark .metric-item {
  background: rgba(255, 255, 255, 0.04);
}

.metric-value {
  font-size: 18px;
  font-weight: 700;
  color: var(--color-on-surface, #111827);
  margin-bottom: 4px;
}

:root.dark .metric-value {
  color: #f3f4f6;
}

.metric-label {
  font-size: 11px;
  color: var(--color-on-surface-muted, #9ca3af);
}

/* 调试界面 - 错误内容 */
.error-content {
  padding: 12px;
  border-radius: 8px;
  background: rgba(239, 68, 68, 0.08);
  color: #dc2626;
  font-size: 12px;
  line-height: 1.5;
}

:root.dark .error-content {
  background: rgba(239, 68, 68, 0.15);
  color: #fca5a5;
}

/* 调试界面 - 结果预览 */
.result-pre {
  margin: 0;
  padding: 12px;
  font-size: 12px;
  font-family: ui-monospace, monospace;
  line-height: 1.5;
  white-space: pre-wrap;
  word-break: break-word;
  background: rgba(128, 128, 128, 0.04);
  border-radius: 8px;
  color: var(--color-on-surface, #374151);
}

:root.dark .result-pre {
  background: rgba(0, 0, 0, 0.3);
  color: #d1d5db;
}
</style>
