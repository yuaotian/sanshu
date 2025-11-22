<script setup lang="ts">
import { invoke } from '@tauri-apps/api/core'
import { useMessage } from 'naive-ui'
import { onMounted, ref, watch, computed } from 'vue'
import { useMcpToolsReactive } from '../../composables/useMcpTools'
import { useAcemcpSync } from '../../composables/useAcemcpSync'

// 使用全局MCP工具状态
const {
  mcpTools,
  loading,
  loadMcpTools,
  toggleTool: globalToggleTool,
  toolStats,
} = useMcpToolsReactive()

// 使用 Acemcp 同步状态管理
const {
  currentProjectStatus,
  autoIndexEnabled,
  watchingProjects,
  statusSummary,
  statusIcon,
  isIndexing,
  fetchAllStatus,
  fetchProjectStatus,
  triggerIndexUpdate,
  fetchAutoIndexEnabled,
  setAutoIndexEnabled,
  fetchWatchingProjects,
  setCurrentProject,
} = useAcemcpSync()

const needsReconnect = ref(false)
// 工具配置弹窗状态
const showToolConfigModal = ref(false)
const currentToolId = ref('')
const acemcpConfig = ref({
  base_url: '',
  token: '',
  batch_size: 10,
  max_lines_per_blob: 800,
  text_extensions: ['.py', '.js', '.ts', '.jsx', '.tsx', '.java', '.go', '.rs', '.cpp', '.c', '.h', '.hpp', '.cs', '.rb', '.php', '.md', '.txt', '.json', '.yaml', '.yml', '.toml', '.xml', '.html', '.css', '.scss', '.sql', '.sh', '.bash'],
  exclude_patterns: ['.venv', 'venv', '.env', 'env', 'node_modules', '.next', '.nuxt', '.output', 'out', '.cache', '.turbo', '.vercel', '.netlify', '.swc', '.vite', '.parcel-cache', '.sass-cache', '.eslintcache', '.stylelintcache', 'coverage', '.nyc_output', 'tmp', 'temp', '.tmp', '.temp', '.git', '.svn', '.hg', '__pycache__', '.pytest_cache', '.mypy_cache', '.tox', '.eggs', '*.egg-info', 'dist', 'build', '.idea', '.vscode', '.DS_Store', '*.pyc', '*.pyo', '*.pyd', '.Python', 'pip-log.txt', 'pip-delete-this-directory.txt', '.coverage', 'htmlcov', '.gradle', 'target', 'bin', 'obj'],
})

// 建议项（用于多选 + 标签）
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

// Naive UI 消息和模态框实例
const message = useMessage()

// 工具调试状态
const debugProjectRoot = ref('')
const debugQuery = ref('')
const debugResult = ref('')
const debugLoading = ref(false)

// 索引管理相关状态
const indexManagementProjectRoot = ref('')
const indexingInProgress = ref(false)

// 格式化时间
function formatTime(timeStr: string | null): string {
  if (!timeStr)
    return '从未'
  try {
    const date = new Date(timeStr)
    return date.toLocaleString('zh-CN', {
      year: 'numeric',
      month: '2-digit',
      day: '2-digit',
      hour: '2-digit',
      minute: '2-digit',
      second: '2-digit',
    })
  }
  catch {
    return '无效时间'
  }
}

// 计算目录统计摘要
const directorySummary = computed(() => {
  if (!currentProjectStatus.value?.directory_stats)
    return []

  return Object.entries(currentProjectStatus.value.directory_stats)
    .map(([dir, [total, indexed]]) => ({
      directory: dir,
      total,
      indexed,
      percentage: total > 0 ? Math.round((indexed / total) * 100) : 0,
    }))
    .sort((a, b) => b.total - a.total) // 按文件数降序
    .slice(0, 10) // 只显示前 10 个目录
})

async function runToolDebug() {
  try {
    if (!debugProjectRoot.value || !debugQuery.value) {
      message.warning('请填写项目根路径与查询语句')
      return
    }
    // 基础校验 API 地址
    if (!acemcpConfig.value.base_url || !/^https?:\/\//i.test(acemcpConfig.value.base_url)) {
      message.error('API端点URL无效，请以 http:// 或 https:// 开头')
      return
    }
    debugLoading.value = true

    // 清空之前的结果
    debugResult.value = ''

    // 使用调试命令执行搜索
    const result = await invoke('debug_acemcp_search', {
      projectRootPath: debugProjectRoot.value,
      query: debugQuery.value,
    }) as { success: boolean, result?: string, error?: string }

    // 设置结果（原样输出）
    if (result.success && result.result) {
      debugResult.value = result.result
    }
    else if (result.error) {
      debugResult.value = result.error
    }
    else {
      debugResult.value = result.result || ''
    }

    if (result.success) {
      message.success('调试执行成功', { duration: 3000 })
    }
    else {
      message.error(result.error || '调试执行失败', { duration: 5000 })
    }
  }
  catch (e: any) {
    const errorMsg = typeof e === 'string' ? e : (e?.message || String(e))
    debugResult.value = `调试失败: ${errorMsg}`
    message.error(`调试失败: ${errorMsg}`, { duration: 5000 })
  }
  finally {
    debugLoading.value = false
  }
}

// 切换工具启用状态（包装全局方法）
async function toggleTool(toolId: string) {
  try {
    const result = await globalToggleTool(toolId)

    // 显示重连提示
    if (result.needsReconnect) {
      needsReconnect.value = true
    }

    if (message) {
      message.warning('MCP工具配置已更新，请在MCP客户端中重连服务')
    }
  }
  catch (err) {
    if (message) {
      message.error(`更新MCP工具状态失败: ${err}`)
    }
  }
}

// 打开工具配置弹窗
async function openToolConfig(toolId: string) {
  currentToolId.value = toolId

  // 如果是代码搜索工具，加载当前配置
  if (toolId === 'sou') {
    await loadAcemcpConfig()
  }

  showToolConfigModal.value = true
}

// 加载acemcp配置
async function loadAcemcpConfig() {
  try {
    const config = await invoke('get_acemcp_config') as {
      base_url?: string
      token?: string
      batch_size: number
      max_lines_per_blob: number
      text_extensions: string[]
      exclude_patterns: string[]
    }

    acemcpConfig.value = {
      base_url: config.base_url || '',
      token: config.token || '',
      batch_size: config.batch_size,
      max_lines_per_blob: config.max_lines_per_blob,
      text_extensions: config.text_extensions,
      exclude_patterns: config.exclude_patterns,
    }

    // 确保选中值都在选项中可见
    const extSet = new Set(extOptions.value.map(o => o.value))
    for (const v of acemcpConfig.value.text_extensions) {
      if (!extSet.has(v))
        extOptions.value.push({ label: v, value: v })
    }
    const exSet = new Set(excludeOptions.value.map(o => o.value))
    for (const v of acemcpConfig.value.exclude_patterns) {
      if (!exSet.has(v))
        excludeOptions.value.push({ label: v, value: v })
    }
  }
  catch (err) {
    if (message) {
      message.error(`加载acemcp配置失败: ${err}`)
    }
  }
}

// 获取当前工具名称
function getCurrentToolName() {
  const tool = mcpTools.value.find(t => t.id === currentToolId.value)
  return tool ? tool.name : ''
}

// 保存acemcp配置
async function saveAcemcpConfig() {
  try {
    if (!acemcpConfig.value.base_url || !/^https?:\/\//i.test(acemcpConfig.value.base_url)) {
      message.error('API端点URL无效，请以 http:// 或 https:// 开头')
      return
    }
    // 多选组件直接双向绑定到数组，无需额外同步
    await invoke('save_acemcp_config', {
      args: {
        baseUrl: acemcpConfig.value.base_url,
        token: acemcpConfig.value.token,
        batchSize: acemcpConfig.value.batch_size,
        maxLinesPerBlob: acemcpConfig.value.max_lines_per_blob,
        textExtensions: acemcpConfig.value.text_extensions,
        excludePatterns: acemcpConfig.value.exclude_patterns,
      },
    })

    message.success('acemcp配置已保存')
    // 不自动关闭弹窗，便于继续编辑/调试
  }
  catch (err) {
    if (message) {
      message.error(`保存acemcp配置失败: ${err}`)
    }
  }
}

// 保存当前工具配置
async function saveCurrentToolConfig() {
  if (currentToolId.value === 'sou') {
    await saveAcemcpConfig()
  }
  // 未来可以添加其他工具的保存逻辑
}

// 测试连接
async function testConnection() {
  let loadingMessage: any = null
  try {
    loadingMessage = message.loading('正在测试连接...', { duration: 0 })

    const result = await invoke('test_acemcp_connection', {
      args: {
        baseUrl: acemcpConfig.value.base_url,
        token: acemcpConfig.value.token,
      },
    }) as { success: boolean, logs: string[], message: string }

    // 关闭加载提示
    if (loadingMessage) {
      loadingMessage.destroy()
      loadingMessage = null
    }

    if (result.success) {
      message.success(result.message, { duration: 3000 })
    }
    else {
      message.error(result.message, { duration: 5000 })
    }
  }
  catch (err) {
    // 关闭加载提示
    if (loadingMessage) {
      loadingMessage.destroy()
      loadingMessage = null
    }

    const errorMsg = typeof err === 'string' ? err : String(err)
    if (message) {
      message.error(`连接测试失败: ${errorMsg}`, { duration: 5000 })
    }
  }
}

// 查看日志
async function viewLogs() {
  try {
    const lines = await invoke('read_acemcp_logs') as string[]
    if (lines.length > 0) {
      const logText = lines.join('\n')
      if (typeof navigator !== 'undefined' && navigator.clipboard) {
        await navigator.clipboard.writeText(logText)
        message.success(`日志已复制到剪贴板（共 ${lines.length} 行，最近1000行）`)
      }
    }
    else {
      message.info('日志文件为空')
    }
  }
  catch (e) {
    const errorMsg = typeof e === 'string' ? e : (e?.message || String(e))
    message.error(`加载日志失败: ${errorMsg}`)
    console.error('加载日志失败:', e)
  }
}

// 清除缓存
async function clearCache() {
  try {
    message.loading('正在清除缓存...')
    const result = await invoke('clear_acemcp_cache') as string
    message.success(result)
  }
  catch (err) {
    if (message) {
      message.error(`清除缓存失败: ${err}`)
    }
  }
}

// 手动触发索引
async function manualTriggerIndex() {
  if (!indexManagementProjectRoot.value) {
    message.error('请输入项目根路径')
    return
  }

  indexingInProgress.value = true
  try {
    const result = await triggerIndexUpdate(indexManagementProjectRoot.value)
    message.success(result)
    // 刷新状态
    await fetchProjectStatus(indexManagementProjectRoot.value)
    setCurrentProject(indexManagementProjectRoot.value)
  }
  catch (err) {
    message.error(String(err))
  }
  finally {
    indexingInProgress.value = false
  }
}

// 切换自动索引开关
async function toggleAutoIndex() {
  try {
    await setAutoIndexEnabled(!autoIndexEnabled.value)
    message.success(`自动索引已${autoIndexEnabled.value ? '启用' : '禁用'}`)
  }
  catch (err) {
    message.error(String(err))
  }
}

// 刷新索引状态
async function refreshIndexStatus() {
  try {
    await fetchAllStatus()
    await fetchAutoIndexEnabled()
    await fetchWatchingProjects()
    if (indexManagementProjectRoot.value) {
      await fetchProjectStatus(indexManagementProjectRoot.value)
      setCurrentProject(indexManagementProjectRoot.value)
    }
    message.success('状态已刷新')
  }
  catch (err) {
    message.error(`刷新状态失败: ${err}`)
  }
}

onMounted(async () => {
  try {
    await loadMcpTools()
    // 初始化索引状态
    await fetchAutoIndexEnabled()
    await fetchWatchingProjects()
  }
  catch (err) {
    if (message) {
      message.error(`加载MCP工具配置失败: ${err}`)
    }
  }
})

// 规范化：保证扩展名格式（小写、以点开头）
watch(() => acemcpConfig.value.text_extensions, (list) => {
  const norm = Array.from(new Set((list || []).map((s) => {
    const t = (s || '').trim().toLowerCase()
    if (!t)
      return ''
    return t.startsWith('.') ? t : `.${t}`
  }).filter(Boolean)))
  if (norm.join(',') !== (list || []).join(',')) {
    acemcpConfig.value.text_extensions = norm
  }
}, { deep: true })

// 关闭弹窗时自动断开实时日志连接
</script>

<template>
  <div class="max-w-3xl mx-auto tab-content">
    <n-space vertical size="large">
      <!-- MCP服务重连提示 -->
      <n-alert v-if="needsReconnect" title="需要重连MCP服务" type="warning" closable @close="needsReconnect = false">
        <template #icon>
          <div class="i-carbon-connection-signal text-lg" />
        </template>
        MCP工具配置已更改，请在您的MCP客户端中重新连接寸止服务以使更改生效。
      </n-alert>

      <!-- 加载状态 -->
      <div v-if="loading" class="text-center py-8">
        <n-spin size="medium" />
        <div class="mt-2 text-sm opacity-60">
          加载MCP工具配置中...
        </div>
      </div>

      <!-- MCP工具配置卡片 -->
      <n-card
        v-for="tool in mcpTools" v-else :key="tool.id" size="small" :class="{ 'opacity-60': !tool.enabled }"
        class="shadow-sm hover:shadow-md transition-shadow duration-200"
      >
        <!-- 卡片头部 -->
        <template #header>
          <div class="flex items-center justify-between">
            <!-- 左侧内容区域 - 允许收缩但不会挤压右侧 -->
            <div class="flex items-center gap-3 flex-1 min-w-0">
              <!-- 图标 -->
              <div
                class="w-10 h-10 rounded-lg flex items-center justify-center flex-shrink-0"
                :class="[tool.icon_bg, tool.dark_icon_bg]"
              >
                <div :class="tool.icon" />
              </div>

              <!-- 标题和副标题 -->
              <div class="flex-1 min-w-0">
                <n-space align="center">
                  <div class="text-lg font-medium tracking-tight">
                    {{ tool.name }}
                  </div>
                  <!-- 状态标签 -->
                  <n-tag v-if="!tool.can_disable" type="info" size="small" :bordered="false">
                    必需
                  </n-tag>
                  <n-tag v-else-if="tool.enabled" type="success" size="small" :bordered="false">
                    已启用
                  </n-tag>
                  <n-tag v-else type="default" size="small" :bordered="false">
                    已禁用
                  </n-tag>
                </n-space>
                <n-tooltip :show-arrow="false" placement="bottom-start" :style="{ maxWidth: '400px' }">
                  <template #trigger>
                    <div class="text-sm opacity-60 font-normal mt-1 truncate cursor-help">
                      {{ tool.description }}
                    </div>
                  </template>
                  <div class="text-sm leading-relaxed">
                    {{ tool.description }}
                  </div>
                </n-tooltip>
              </div>
            </div>

            <!-- 右侧操作按钮区域 - 固定宽度，不会被挤压 -->
            <div class="flex flex-shrink-0 ml-4 gap-2 items-center">
              <!-- 设置按钮 - 只有有配置的工具才显示 -->
              <n-button
                v-if="tool.can_disable && tool.has_config" size="small" quaternary circle
                @click="openToolConfig(tool.id)"
              >
                <template #icon>
                  <div class="i-carbon-settings-adjust w-4 h-4" />
                </template>
              </n-button>

              <!-- 开关 -->
              <n-switch
                v-if="tool.can_disable" :value="tool.enabled" size="small"
                @update:value="toggleTool(tool.id)"
              />
            </div>
          </div>
        </template>
      </n-card>

      <!-- 底部统计 - 增强可见性 -->
      <div class="text-center py-2">
        <span class="text-sm text-gray-500 dark:text-gray-400 font-medium">
          {{ toolStats.enabled }} / {{ toolStats.total }} 个工具已启用
        </span>
      </div>
    </n-space>

    <!-- 工具配置弹窗 -->
    <n-modal
      v-model:show="showToolConfigModal" preset="card" :closable="true" :mask-closable="true"
      :title="`${getCurrentToolName()} 工具配置`" style="width: 800px" :bordered="false" size="huge"
    >
      <!-- 代码搜索工具配置 -->
      <div v-if="currentToolId === 'sou'">
        <n-tabs type="line" animated>
          <!-- 基础配置标签页 -->
          <n-tab-pane name="basic" tab="基础配置">
            <n-space vertical size="large">
              <n-form-item label="API端点URL">
                <n-input v-model:value="acemcpConfig.base_url" placeholder="https://api.example.com" clearable />
              </n-form-item>

              <n-form-item label="认证令牌">
                <n-input
                  v-model:value="acemcpConfig.token" type="password" show-password-on="click"
                  placeholder="your-token-here" clearable
                />
              </n-form-item>

              <n-form-item label="批处理大小">
                <n-input-number v-model:value="acemcpConfig.batch_size" :min="1" :max="100" placeholder="10" />
              </n-form-item>

              <n-form-item label="最大行数/块">
                <n-input-number
                  v-model:value="acemcpConfig.max_lines_per_blob" :min="100" :max="5000"
                  placeholder="800"
                />
              </n-form-item>
            </n-space>
          </n-tab-pane>

          <!-- 高级配置标签页 -->
          <n-tab-pane name="advanced" tab="高级配置">
            <n-space vertical size="large">
              <n-form-item label="文件扩展名">
                <n-select
                  v-model:value="acemcpConfig.text_extensions" :options="extOptions" multiple tag filterable
                  clearable placeholder="选择或输入扩展名，如 .py"
                />
                <template #feedback>
                  建议小写，以点开头；重复项自动去重。
                </template>
              </n-form-item>

              <n-form-item label="排除模式">
                <n-select
                  v-model:value="acemcpConfig.exclude_patterns" :options="excludeOptions" multiple tag
                  filterable clearable placeholder="选择或输入排除模式，如 node_modules 或 *.pyc"
                />
                <template #feedback>
                  支持通配符；从常见项中选择或输入自定义模式。
                </template>
              </n-form-item>
            </n-space>
          </n-tab-pane>

          <!-- 日志和调试标签页 -->
          <n-tab-pane name="debug" tab="日志和调试">
            <n-space vertical size="large">
              <n-alert type="info" title="日志和调试功能">
                <template #icon>
                  <div class="i-carbon-document-text" />
                </template>
                代码搜索工具会自动记录操作日志，包括索引过程、搜索请求和错误信息。日志文件位于 ~/.cunzhi/log/acemcp.log
              </n-alert>

              <!-- 统一的日志和调试区域 -->
              <n-card size="small">
                <template #header>
                  <div class="flex items-center justify-between">
                    <div class="font-medium">
                      日志和调试
                    </div>
                    <n-space size="small">
                      <n-button size="small" @click="testConnection">
                        <template #icon>
                          <div class="i-carbon-connection-signal w-4 h-4" />
                        </template>
                        测试连接
                      </n-button>
                      <n-button size="small" @click="viewLogs">
                        <template #icon>
                          <div class="i-carbon-activity w-4 h-4" />
                        </template>
                        查看日志
                      </n-button>
                      <n-button size="small" @click="clearCache">
                        <template #icon>
                          <div class="i-carbon-trash-can w-4 h-4" />
                        </template>
                        清除缓存
                      </n-button>
                    </n-space>
                  </div>
                </template>

                <n-space vertical size="large">
                  <!-- 调试输入区域 -->
                  <n-collapse :default-expanded-names="['debug']">
                    <n-collapse-item name="debug" title="代码搜索调试">
                      <template #header-extra>
                        <n-tag size="small" type="info" :bordered="false">
                          调试工具
                        </n-tag>
                      </template>
                      <n-space vertical size="medium">
                        <n-form-item label="项目根路径" :show-feedback="false">
                          <n-input
                            v-model:value="debugProjectRoot"
                            placeholder="/abs/path/to/your/project (使用正斜杠)"
                            clearable
                          />
                        </n-form-item>
                        <n-form-item label="查询语句" :show-feedback="false">
                          <n-input
                            v-model:value="debugQuery"
                            type="textarea"
                            :autosize="{ minRows: 2, maxRows: 4 }"
                            placeholder="例如：日志配置初始化 或 用户认证登录"
                            clearable
                          />
                        </n-form-item>
                        <n-space>
                          <n-button
                            type="primary"
                            :loading="debugLoading"
                            @click="runToolDebug"
                          >
                            <template #icon>
                              <div class="i-carbon-play w-4 h-4" />
                            </template>
                            运行调试
                          </n-button>
                          <n-button :disabled="!debugResult" @click="debugResult = ''">
                            清空结果
                          </n-button>
                        </n-space>
                        <n-form-item v-if="debugResult" label="搜索结果" :show-feedback="false">
                          <n-input
                            v-model:value="debugResult"
                            type="textarea"
                            :autosize="{ minRows: 4, maxRows: 10 }"
                            readonly
                            class="result-textarea"
                          />
                        </n-form-item>
                      </n-space>
                    </n-collapse-item>
                  </n-collapse>
                </n-space>
              </n-card>

              <n-alert type="warning" title="使用提示">
                <template #icon>
                  <div class="i-carbon-warning" />
                </template>
                <ul class="text-sm space-y-1">
                  <li>• 测试连接：验证 API 端点和令牌配置是否正确</li>
                  <li>• 运行调试：执行完整的代码索引和搜索流程，查看详细日志</li>
                  <li>• 索引过程是增量式的，只处理新增或修改的文件</li>
                  <li>• 大文件会自动分割成多个块进行处理</li>
                </ul>
              </n-alert>
            </n-space>
          </n-tab-pane>

          <!-- 索引管理标签页 -->
          <n-tab-pane name="index-management" tab="索引管理">
            <n-space vertical size="large">
              <!-- 全局设置卡片 -->
              <n-card size="small" title="全局设置">
                <template #header-extra>
                  <n-button size="small" @click="refreshIndexStatus">
                    <template #icon>
                      <div class="i-carbon-renew w-4 h-4" />
                    </template>
                    刷新状态
                  </n-button>
                </template>

                <n-space vertical size="medium">
                  <!-- 自动索引开关 -->
                  <div class="flex items-center justify-between">
                    <div class="flex items-center gap-2">
                      <div class="i-carbon-automatic w-5 h-5 text-blue-500" />
                      <div>
                        <div class="font-medium">
                          自动索引
                        </div>
                        <div class="text-sm opacity-60">
                          文件变更时自动更新索引（1.5秒防抖）
                        </div>
                      </div>
                    </div>
                    <n-switch :value="autoIndexEnabled" @update:value="toggleAutoIndex" />
                  </div>

                  <!-- 监听中的项目 -->
                  <n-divider />
                  <div>
                    <div class="font-medium mb-2">
                      监听中的项目 ({{ watchingProjects.length }})
                    </div>
                    <div v-if="watchingProjects.length === 0" class="text-sm opacity-60">
                      暂无监听中的项目
                    </div>
                    <n-space v-else vertical size="small">
                      <n-tag
                        v-for="project in watchingProjects" :key="project" type="success" size="small"
                        :bordered="false"
                      >
                        <template #icon>
                          <div class="i-carbon-view w-4 h-4" />
                        </template>
                        {{ project }}
                      </n-tag>
                    </n-space>
                  </div>
                </n-space>
              </n-card>

              <!-- 项目索引状态卡片 -->
              <n-card size="small" title="项目索引状态">
                <n-space vertical size="medium">
                  <!-- 项目路径输入 -->
                  <n-form-item label="项目根路径" :show-feedback="false">
                    <n-input
                      v-model:value="indexManagementProjectRoot"
                      placeholder="/abs/path/to/your/project (使用正斜杠)"
                      clearable
                      @blur="() => {
                        if (indexManagementProjectRoot) {
                          fetchProjectStatus(indexManagementProjectRoot)
                          setCurrentProject(indexManagementProjectRoot)
                        }
                      }"
                    />
                  </n-form-item>

                  <!-- 操作按钮 -->
                  <n-space>
                    <n-button
                      type="primary"
                      :loading="indexingInProgress"
                      :disabled="!indexManagementProjectRoot"
                      @click="manualTriggerIndex"
                    >
                      <template #icon>
                        <div class="i-carbon-play w-4 h-4" />
                      </template>
                      立即索引
                    </n-button>
                    <n-button
                      :disabled="!indexManagementProjectRoot"
                      @click="clearCache"
                    >
                      <template #icon>
                        <div class="i-carbon-trash-can w-4 h-4" />
                      </template>
                      清除缓存
                    </n-button>
                  </n-space>

                  <!-- 状态展示 -->
                  <n-divider v-if="currentProjectStatus" />
                  <div v-if="currentProjectStatus">
                    <!-- 状态摘要 -->
                    <div class="flex items-center gap-3 mb-4">
                      <div :class="statusIcon" class="w-6 h-6" />
                      <div>
                        <div class="font-medium text-lg">
                          {{ statusSummary }}
                        </div>
                        <div class="text-sm opacity-60">
                          进度: {{ currentProjectStatus.progress }}%
                        </div>
                      </div>
                    </div>

                    <!-- 文件统计 -->
                    <n-grid :cols="2" :x-gap="12" :y-gap="12">
                      <n-gi>
                        <n-statistic label="总文件数" :value="currentProjectStatus.total_files">
                          <template #prefix>
                            <div class="i-carbon-document w-4 h-4" />
                          </template>
                        </n-statistic>
                      </n-gi>
                      <n-gi>
                        <n-statistic label="已索引" :value="currentProjectStatus.indexed_files">
                          <template #prefix>
                            <div class="i-carbon-checkmark-filled w-4 h-4 text-green-500" />
                          </template>
                        </n-statistic>
                      </n-gi>
                      <n-gi>
                        <n-statistic label="待处理" :value="currentProjectStatus.pending_files">
                          <template #prefix>
                            <div class="i-carbon-time w-4 h-4 text-blue-500" />
                          </template>
                        </n-statistic>
                      </n-gi>
                      <n-gi>
                        <n-statistic label="失败" :value="currentProjectStatus.failed_files">
                          <template #prefix>
                            <div class="i-carbon-warning-filled w-4 h-4 text-red-500" />
                          </template>
                        </n-statistic>
                      </n-gi>
                    </n-grid>

                    <!-- 时间信息 -->
                    <n-divider />
                    <n-space vertical size="small">
                      <div class="text-sm">
                        <span class="opacity-60">最后成功时间:</span>
                        <span class="ml-2">{{ formatTime(currentProjectStatus.last_success_time) }}</span>
                      </div>
                      <div v-if="currentProjectStatus.last_failure_time" class="text-sm">
                        <span class="opacity-60">最后失败时间:</span>
                        <span class="ml-2">{{ formatTime(currentProjectStatus.last_failure_time) }}</span>
                      </div>
                      <div v-if="currentProjectStatus.last_error" class="text-sm text-red-500">
                        <span class="opacity-60">错误信息:</span>
                        <span class="ml-2">{{ currentProjectStatus.last_error }}</span>
                      </div>
                    </n-space>

                    <!-- 目录统计 -->
                    <n-divider v-if="directorySummary.length > 0" />
                    <div v-if="directorySummary.length > 0">
                      <div class="font-medium mb-2">
                        目录统计 (前10)
                      </div>
                      <n-space vertical size="small">
                        <div
                          v-for="dir in directorySummary" :key="dir.directory"
                          class="flex items-center justify-between text-sm"
                        >
                          <div class="flex-1 truncate opacity-80">
                            {{ dir.directory }}
                          </div>
                          <div class="flex items-center gap-2 ml-2">
                            <span class="opacity-60">{{ dir.indexed }}/{{ dir.total }}</span>
                            <n-progress
                              type="line"
                              :percentage="dir.percentage"
                              :show-indicator="false"
                              :height="4"
                              style="width: 60px"
                            />
                          </div>
                        </div>
                      </n-space>
                    </div>
                  </div>

                  <!-- 未选择项目提示 -->
                  <n-empty v-else description="请输入项目根路径以查看索引状态" />
                </n-space>
              </n-card>

              <!-- 使用提示 -->
              <n-alert type="info" title="索引管理说明">
                <template #icon>
                  <div class="i-carbon-information" />
                </template>
                <ul class="text-sm space-y-1">
                  <li>• 首次搜索时会自动启动文件监听（如果全局开关已启用）</li>
                  <li>• 文件变更后会自动触发索引更新（1.5秒防抖）</li>
                  <li>• 索引是增量式的，只处理新增或修改的文件</li>
                  <li>• 可以手动触发索引更新或清除缓存重建</li>
                </ul>
              </n-alert>
            </n-space>
          </n-tab-pane>
        </n-tabs>
      </div>

      <!-- 其他工具的配置占位 -->
      <div v-else class="text-center py-8">
        <n-empty description="此工具暂无配置选项" />
      </div>

      <template #footer>
        <n-space justify="end">
          <n-button @click="showToolConfigModal = false">
            取消
          </n-button>
          <n-button v-if="currentToolId === 'sou'" type="primary" @click="saveCurrentToolConfig">
            保存配置
          </n-button>
        </n-space>
      </template>
    </n-modal>
  </div>
</template>

<style scoped>
.result-textarea {
  font-family: ui-monospace, SFMono-Regular, Menlo, Monaco, Consolas, "Liberation Mono", "Courier New", monospace;
}
</style>
