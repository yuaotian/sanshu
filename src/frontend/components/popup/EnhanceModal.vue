<script setup lang="ts">
// 修复重复的 script setup 声明，避免 SFC 解析错误
import { invoke } from '@tauri-apps/api/core'
import { listen, type UnlistenFn } from '@tauri-apps/api/event'
import { useMediaQuery } from '@vueuse/core'
import { useMessage } from 'naive-ui'
import { computed, onMounted, onUnmounted, ref, watch } from 'vue'
import type { CustomPrompt } from '../../types/popup'
import { buildConditionalContext } from '../../utils/conditionalContext'
import EnhanceConfigPanel from './enhance/EnhanceConfigPanel.vue'
import EnhancePreview from './enhance/EnhancePreview.vue'
import EnhanceResult from './enhance/EnhanceResult.vue'

interface Props {
  show: boolean
  originalPrompt: string
  projectRootPath?: string
  currentFilePath?: string
}

interface Emits {
  'update:show': [value: boolean]
  'confirm': [enhancedPrompt: string]
  'cancel': []
}

interface EnhanceStreamEvent {
  request_id: string
  event_type: 'chunk' | 'complete' | 'error'
  chunk?: string
  accumulated_text?: string
  enhanced_prompt?: string
  error?: string
  progress: number
}

interface EnhanceResponse {
  enhanced_prompt: string
  original_prompt: string
  success: boolean
  error?: string | null
  blob_count?: number
  history_count?: number
  project_root_path?: string | null
  blob_source_root?: string | null
  request_id?: string | null
}

interface EnhanceConfig {
  includeContext: boolean
  includeHistory: boolean
  selectedHistoryIds: string[]
  useDefaultRule: boolean
  customRule: string
}

interface ChatHistoryEntry {
  id: string
  user_input: string
  ai_response_summary: string
  timestamp: string
  source?: string
}

const props = defineProps<Props>()
const emit = defineEmits<Emits>()

const message = useMessage()
const isMobile = useMediaQuery('(max-width: 640px)')

const DEFAULT_RULE = '请确保增强后的提示词以中文为主要输出语言'
const CUSTOM_RULE_MAX = 200
// 中文注释：限制最终提示词最大长度，避免超时
const MAX_PROMPT_LENGTH = 10000

// 状态
const isEnhancing = ref(false)
const streamContent = ref('')
const enhancedPrompt = ref('')
const progress = ref(0)
const errorMessage = ref('')
const hasCompleted = ref(false)
// 中文注释：用于关联流式事件与当前请求，避免并发串扰
const activeRequestId = ref<string | null>(null)

const config = ref<EnhanceConfig>({
  includeContext: false,
  includeHistory: true,
  selectedHistoryIds: [],
  useDefaultRule: true,
  customRule: '',
})

const customRuleInput = ref('')
const conditionalPrompts = ref<CustomPrompt[]>([])
const historyEntries = ref<ChatHistoryEntry[]>([])
const historyLoading = ref(false)
const historyError = ref('')
const historySelectionTouched = ref(false)
// 后端诊断信息（项目路径与上下文数量）
const blobCount = ref<number | null>(null)
const historyCount = ref<number | null>(null)
const responseProjectRoot = ref('')
const blobSourceRoot = ref('')

// 事件监听器
let unlisten: UnlistenFn | null = null
let customRuleTimer: number | undefined

// 计算属性
const corePrompt = computed(() => props.originalPrompt?.trim() ?? '')
const coreCharCount = computed(() => corePrompt.value.length)
const contextText = computed(() => buildConditionalContext(conditionalPrompts.value))

const finalPrompt = computed(() => {
  if (!corePrompt.value) {
    return ''
  }

  const parts: string[] = []
  if (config.value.includeContext && contextText.value.trim()) {
    parts.push(contextText.value.trim())
  }
  parts.push(corePrompt.value)

  const rules: string[] = []
  if (config.value.useDefaultRule) {
    rules.push(DEFAULT_RULE)
  }
  if (config.value.customRule.trim()) {
    rules.push(config.value.customRule.trim())
  }
  if (rules.length > 0) {
    parts.push(`---\n增强规则：\n${rules.join('\n')}`)
  }

  return parts.join('\n\n')
})

// 优先使用后端回显的项目路径，确保诊断信息一致
const displayProjectRoot = computed(() => responseProjectRoot.value || props.projectRootPath || '')
const displayBlobSourceRoot = computed(() => blobSourceRoot.value || '')

const canConfirm = computed(() => hasCompleted.value && enhancedPrompt.value.length > 0)
const statusText = computed(() => {
  if (errorMessage.value) {
    return '增强失败'
  }
  if (hasCompleted.value) {
    return '增强完成'
  }
  if (isEnhancing.value) {
    return `增强中... ${progress.value}%`
  }
  return '准备就绪'
})

// 处理配置区开关变化
function handleIncludeContextChange(value: boolean) {
  config.value.includeContext = value
  if (value) {
    loadConditionalPrompts()
  }
}

function handleIncludeHistoryChange(value: boolean) {
  config.value.includeHistory = value
  if (value) {
    loadHistoryEntries()
  }
}

function handleUseDefaultRuleChange(value: boolean) {
  config.value.useDefaultRule = value
}

function handleCustomRuleInputChange(value: string) {
  customRuleInput.value = value
}

// 生成请求 ID（用于事件关联与取消）
function createRequestId() {
  // 中文注释：优先使用浏览器原生的随机 ID
  if (typeof crypto !== 'undefined' && 'randomUUID' in crypto) {
    return crypto.randomUUID()
  }
  // 中文注释：降级方案，保证有唯一性
  return `${Date.now()}_${Math.random().toString(16).slice(2)}`
}

// 取消当前增强请求（用于关闭弹窗时中断后端任务）
async function cancelActiveRequest() {
  if (!activeRequestId.value || !isEnhancing.value) {
    return
  }
  try {
    await invoke('cancel_enhance_request', { requestId: activeRequestId.value })
  }
  catch (error) {
    console.warn('取消增强请求失败:', error)
  }
}

// 复制增强结果
async function handleCopyResult() {
  if (!enhancedPrompt.value) {
    return
  }
  try {
    // 中文注释：剪贴板 API 不可用时给出提示
    if (!navigator.clipboard) {
      message.error('当前环境不支持自动复制')
      return
    }
    await navigator.clipboard.writeText(enhancedPrompt.value)
    message.success('已复制增强结果')
  }
  catch (error) {
    console.error('复制增强结果失败:', error)
    message.error('复制失败，请手动选择文本')
  }
}

// 统一重置增强状态
function resetEnhanceState() {
  isEnhancing.value = false
  streamContent.value = ''
  enhancedPrompt.value = ''
  progress.value = 0
  errorMessage.value = ''
  hasCompleted.value = false
  activeRequestId.value = null
  blobCount.value = null
  historyCount.value = null
  responseProjectRoot.value = ''
  blobSourceRoot.value = ''
}

// 重置配置（保持默认值）
function resetConfigState() {
  config.value = {
    includeContext: false,
    includeHistory: true,
    selectedHistoryIds: [],
    useDefaultRule: true,
    customRule: '',
  }
  customRuleInput.value = ''
  conditionalPrompts.value = []
  historyEntries.value = []
  historyError.value = ''
  historySelectionTouched.value = false
}

// 加载条件性 prompt 上下文
async function loadConditionalPrompts() {
  try {
    const configData = await invoke('get_custom_prompt_config')
    const promptConfig = configData as any
    const prompts = (promptConfig.prompts || []) as CustomPrompt[]
    conditionalPrompts.value = prompts.filter(prompt => prompt.type === 'conditional')
  }
  catch (error) {
    console.error('加载条件性prompt失败:', error)
    message.error('加载快捷上下文失败')
  }
}

// 加载最近历史记录
async function loadHistoryEntries() {
  if (!props.projectRootPath) {
    historyEntries.value = []
    historyError.value = '未提供项目路径，无法读取历史记录'
    return
  }

  historyLoading.value = true
  historyError.value = ''

  try {
    const entries = await invoke('get_chat_history', {
      projectRootPath: props.projectRootPath,
      count: 5,
    }) as ChatHistoryEntry[]

    historyEntries.value = entries

    // 未被用户修改时，默认全选
    if (!historySelectionTouched.value) {
      config.value.selectedHistoryIds = entries.map(entry => entry.id)
    }
  }
  catch (error) {
    console.error('加载对话历史失败:', error)
    historyError.value = '加载历史记录失败'
  }
  finally {
    historyLoading.value = false
  }
}

// 自定义规则输入防抖
watch(customRuleInput, (value) => {
  if (value.length > CUSTOM_RULE_MAX) {
    customRuleInput.value = value.slice(0, CUSTOM_RULE_MAX)
    return
  }

  if (customRuleTimer) {
    window.clearTimeout(customRuleTimer)
  }

  customRuleTimer = window.setTimeout(() => {
    config.value.customRule = customRuleInput.value.trim()
  }, 300)
})

// 处理历史选择变更
function handleHistorySelectionChange(ids: string[]) {
  historySelectionTouched.value = true
  config.value.selectedHistoryIds = ids
}

// 准备增强：先加载上下文/历史，再触发增强
async function prepareEnhance() {
  if (!corePrompt.value) {
    return
  }

  if (config.value.includeContext) {
    await loadConditionalPrompts()
  }

  if (config.value.includeHistory) {
    await loadHistoryEntries()
  }

  if (props.show) {
    startEnhance()
  }
}

// 开始增强
async function startEnhance() {
  if (isEnhancing.value) {
    return
  }

  // 启动前释放旧监听，避免重复订阅导致多次回调
  if (unlisten) {
    unlisten()
    unlisten = null
  }

  // 重置状态
  resetEnhanceState()

  // 中文注释：限制提示词长度，避免请求超时或被拒绝
  if (finalPrompt.value.length > MAX_PROMPT_LENGTH) {
    const errorText = `提示词过长（>${MAX_PROMPT_LENGTH} 字符），请精简后再试`
    errorMessage.value = errorText
    message.error(errorText)
    return
  }

  isEnhancing.value = true
  const requestId = createRequestId()
  activeRequestId.value = requestId

  try {
    // 进度锁定标志：complete 事件后锁定，防止后续 chunk 事件重置进度
    let completeLock = false

    // 设置事件监听
    unlisten = await listen<EnhanceStreamEvent>('enhance-stream', (event) => {
      const data = event.payload
      if (activeRequestId.value !== requestId || data.request_id !== requestId) {
        return
      }

      switch (data.event_type) {
        case 'chunk':
          // 锁定后忽略 chunk 事件的进度更新，防止进度条重置
          if (!completeLock) {
            if (data.accumulated_text) {
              streamContent.value = data.accumulated_text
            }
            progress.value = data.progress
          }
          break
        case 'complete':
          // 锁定进度，防止后续 chunk 事件影响
          completeLock = true
          if (data.enhanced_prompt) {
            enhancedPrompt.value = data.enhanced_prompt
          }
          if (data.accumulated_text) {
            streamContent.value = data.accumulated_text
          }
          progress.value = 100
          hasCompleted.value = true
          isEnhancing.value = false
          break
        case 'error':
          completeLock = true // 错误状态也锁定
          errorMessage.value = data.error || '未知错误'
          isEnhancing.value = false
          break
      }
    })

    const selectedHistoryIds = config.value.includeHistory
      ? (historySelectionTouched.value ? config.value.selectedHistoryIds : null)
      : null

    // 调用后端增强并记录诊断信息
    const response = await invoke('enhance_prompt_stream', {
      prompt: finalPrompt.value,
      projectRootPath: props.projectRootPath || null,
      currentFilePath: props.currentFilePath || null,
      includeHistory: config.value.includeHistory,
      selectedHistoryIds,
      requestId,
    }) as EnhanceResponse

    if (!response || activeRequestId.value !== requestId) {
      return
    }
    if (response.request_id && response.request_id !== requestId) {
      return
    }

    blobCount.value = typeof response.blob_count === 'number' ? response.blob_count : null
    historyCount.value = typeof response.history_count === 'number' ? response.history_count : null
    responseProjectRoot.value = response.project_root_path || props.projectRootPath || ''
    blobSourceRoot.value = response.blob_source_root || ''

    // 中文注释：事件监听失败时，用响应结果兜底
    if (response.success && response.enhanced_prompt && !hasCompleted.value) {
      enhancedPrompt.value = response.enhanced_prompt
      streamContent.value = response.enhanced_prompt
      progress.value = 100
      hasCompleted.value = true
      isEnhancing.value = false
    }

    if (response.success === false && response.error && !errorMessage.value) {
      errorMessage.value = response.error
      isEnhancing.value = false
    }
  }
  catch (error) {
    if (activeRequestId.value !== requestId) {
      return
    }
    console.error('增强失败:', error)
    errorMessage.value = String(error)
    isEnhancing.value = false
  }
  finally {
    if (activeRequestId.value === requestId) {
      activeRequestId.value = null
    }
  }
}

// 确认使用增强结果
function handleConfirm() {
  if (!canConfirm.value) {
    return
  }
  emit('confirm', enhancedPrompt.value)
  handleClose()
}

// 重试增强
function handleRetry() {
  if (isEnhancing.value) {
    return
  }
  startEnhance()
}

// 取消/关闭
function handleClose() {
  emit('update:show', false)
  emit('cancel')
  cleanup()
}

// 清理
function cleanup() {
  // 中文注释：关闭时尽量取消后端请求，减少资源浪费
  void cancelActiveRequest()
  if (unlisten) {
    unlisten()
    unlisten = null
  }
  if (customRuleTimer) {
    window.clearTimeout(customRuleTimer)
    customRuleTimer = undefined
  }
  resetEnhanceState()
}

// 监听 show 变化，自动开始增强
watch(() => props.show, (newValue) => {
  if (newValue) {
    resetEnhanceState()
    resetConfigState()
    prepareEnhance()
  }
  // 关闭弹窗时清理监听与状态，避免残留
  if (!newValue) {
    cleanup()
  }
})

// 组件挂载
onMounted(() => {
  if (props.show && corePrompt.value) {
    resetEnhanceState()
    resetConfigState()
    prepareEnhance()
  }
})

// 组件卸载时清理
onUnmounted(() => {
  cleanup()
})
</script>

<template>
  <n-modal
    :show="show"
    preset="card"
    :closable="false"
    :mask-closable="!isEnhancing"
    class="w-[640px] max-w-[92vw]"
    @update:show="(val: boolean) => !isEnhancing && emit('update:show', val)"
  >
    <template #header>
      <div class="flex items-center justify-between">
        <div class="flex items-center gap-2 text-sm font-semibold text-slate-700 dark:text-slate-100">
          <div class="i-carbon-magic-wand h-4 w-4 text-amber-500" />
          提示词增强
        </div>
        <n-button
          text
          size="small"
          :disabled="isEnhancing"
          aria-label="关闭提示词增强弹窗"
          title="关闭"
          class="text-slate-500 hover:text-slate-700 dark:text-slate-400 dark:hover:text-slate-200"
          @click="handleClose"
        >
          <div class="i-carbon-close h-4 w-4" />
        </n-button>
      </div>
    </template>

    <div class="space-y-4">
      <!-- 核心输入区 -->
      <div class="rounded-2xl border border-stone-200/80 bg-gradient-to-br from-stone-50/80 to-amber-50/60 p-4 shadow-sm dark:border-slate-700/50 dark:from-slate-900/40 dark:to-slate-800/40">
        <div class="mb-2 flex items-center justify-between text-xs text-slate-500 dark:text-slate-400">
          <div class="flex items-center gap-2">
            <div class="i-carbon-document h-3.5 w-3.5" />
            核心提示词
          </div>
          <span>{{ coreCharCount }} 字符</span>
        </div>
        <div class="max-h-28 overflow-y-auto whitespace-pre-wrap text-sm text-slate-700 dark:text-slate-200">
          {{ corePrompt || '暂无输入内容' }}
        </div>
      </div>

      <!-- 配置区 -->
      <EnhanceConfigPanel
        :include-context="config.includeContext"
        :include-history="config.includeHistory"
        :use-default-rule="config.useDefaultRule"
        :custom-rule-input="customRuleInput"
        :custom-rule-max="CUSTOM_RULE_MAX"
        :history-entries="historyEntries"
        :selected-history-ids="config.selectedHistoryIds"
        :history-loading="historyLoading"
        :history-error="historyError"
        :default-rule-text="DEFAULT_RULE"
        :is-mobile="isMobile"
        @update:include-context="handleIncludeContextChange"
        @update:include-history="handleIncludeHistoryChange"
        @update:use-default-rule="handleUseDefaultRuleChange"
        @update:custom-rule-input="handleCustomRuleInputChange"
        @update:selected-history-ids="handleHistorySelectionChange"
      >
        <template #context-preview>
          <EnhancePreview :core-text="corePrompt" :context-text="contextText" />
        </template>
      </EnhanceConfigPanel>

      <!-- 增强结果区 -->
      <EnhanceResult
        :is-enhancing="isEnhancing"
        :has-completed="hasCompleted"
        :error-message="errorMessage"
        :stream-content="streamContent"
        :enhanced-prompt="enhancedPrompt"
        :progress="progress"
        :status-text="statusText"
        :project-root-path="displayProjectRoot"
        :blob-count="blobCount"
        :history-count="historyCount"
        :blob-source-root="displayBlobSourceRoot"
      />

      <!-- 操作按钮区 -->
      <div class="flex flex-col gap-3 sm:flex-row sm:items-center sm:justify-between">
        <div>
          <n-button
            v-if="errorMessage && !isEnhancing"
            size="small"
            class="w-full !bg-gradient-to-r from-rose-200 to-amber-200 !text-rose-900 shadow-sm hover:from-rose-300 hover:to-amber-300 sm:w-auto"
            @click="handleRetry"
          >
            重试
          </n-button>
          <n-button
            v-else-if="hasCompleted && enhancedPrompt"
            size="small"
            class="w-full bg-white/70 text-slate-600 shadow-sm hover:bg-white sm:w-auto dark:bg-slate-800/60 dark:text-slate-200 dark:hover:bg-slate-800"
            @click="handleCopyResult"
          >
            复制结果
          </n-button>
        </div>
        <div class="flex flex-col gap-2 sm:flex-row sm:items-center">
          <n-button
            size="small"
            class="w-full bg-white/70 text-slate-600 shadow-sm hover:bg-white sm:w-auto dark:bg-slate-800/60 dark:text-slate-200 dark:hover:bg-slate-800"
            @click="handleClose"
          >
            取消
          </n-button>
          <n-button
            size="small"
            :disabled="!canConfirm"
            class="w-full !bg-gradient-to-r from-emerald-200 to-teal-200 !text-emerald-900 shadow-sm hover:from-emerald-300 hover:to-teal-300 sm:w-auto"
            @click="handleConfirm"
          >
            <template #icon>
              <div class="i-carbon-checkmark h-4 w-4" />
            </template>
            确认使用
          </n-button>
        </div>
      </div>
    </div>
  </n-modal>
</template>
