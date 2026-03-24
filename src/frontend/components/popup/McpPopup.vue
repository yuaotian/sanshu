<script setup lang="ts">
import type { McpRequest } from '../../types/popup'
import { invoke } from '@tauri-apps/api/core'
import { listen } from '@tauri-apps/api/event'
import { useDialog, useMessage } from 'naive-ui'
import { computed, onMounted, onUnmounted, ref, watch } from 'vue'

import { useAcemcpSync } from '../../composables/useAcemcpSync'
import { useMcpToolsReactive } from '../../composables/useMcpTools'
import { getContextPolicyStatus, shouldShowPolicyIndicator } from '../../utils/conditionalContext'
import PopupActions from './PopupActions.vue'
import PopupContent from './PopupContent.vue'
import PopupInput from './PopupInput.vue'
import ZhiIndexPanel from './ZhiIndexPanel.vue'

interface AppConfig {
  theme: string
  window: {
    alwaysOnTop: boolean
    width: number
    height: number
    fixed: boolean
  }
  audio: {
    enabled: boolean
    url: string
  }
  reply: {
    enabled: boolean
    prompt: string
  }
}

interface Props {
  request: McpRequest | null
  appConfig: AppConfig
  enhanceEnabled?: boolean
  mockMode?: boolean
  testMode?: boolean
}

interface Emits {
  response: [response: any]
  cancel: []
  themeChange: [theme: string]
  openMainLayout: []
  openMcpToolsTab: [toolId?: string]
  toggleAlwaysOnTop: []
  toggleAudioNotification: []
  updateAudioUrl: [url: string]
  testAudio: []
  stopAudio: []
  testAudioError: [error: any]
  updateWindowSize: [size: { width: number, height: number, fixed: boolean }]
  openIndexStatus: []
}

interface PopupInputUpdatePayload {
  userInput: string
  rawUserInput: string
  conditionalContext: string
  selectedOptions: string[]
  draggedImages: string[]
}

const props = withDefaults(defineProps<Props>(), {
  mockMode: false,
  testMode: false,
  enhanceEnabled: false,
})

const emit = defineEmits<Emits>()

// 使用消息提示
const message = useMessage()
const dialog = useDialog()

// 索引状态管理
const {
  currentProjectStatus,
  statusSummary,
  statusIcon,
  isIndexing,
  startPolling,
  stopPolling,
  setCurrentProject,
  triggerIndexUpdate,
  checkAcemcpConfigured,
} = useAcemcpSync()

// MCP 工具状态
const { mcpTools, loadMcpTools } = useMcpToolsReactive()

// sou 代码搜索工具是否启用
const souEnabled = computed(() => mcpTools.value.some(tool => tool.id === 'sou' && tool.enabled))

// ACE 配置是否完整
const acemcpConfigured = ref(false)

// 索引重新同步加载状态
const resyncLoading = ref(false)

// 响应式状态
const loading = ref(false)
const submitting = ref(false)
const selectedOptions = ref<string[]>([])
const userInput = ref('')
const rawUserInput = ref('')
const conditionalContext = ref('')
const draggedImages = ref<string[]>([])
const inputRef = ref()

// 继续回复配置
const continueReplyEnabled = ref(true)
const continuePrompt = ref('请按照最佳实践继续')

// 计算属性
const isVisible = computed(() => !!props.request)
const hasOptions = computed(() => (props.request?.predefined_options?.length ?? 0) > 0)
const canSubmit = computed(() => {
  if (hasOptions.value) {
    return selectedOptions.value.length > 0 || userInput.value.trim().length > 0 || draggedImages.value.length > 0
  }
  return userInput.value.trim().length > 0 || draggedImages.value.length > 0
})
// 中文注释：本地增强模式始终可用，不再依赖外部 API 配置
const localEnhanceEnabled = true
// 中文注释：本地增强只接受用户原始输入，避免把条件性上下文误当成待改写正文
const canEnhance = computed(() => rawUserInput.value.trim().length > 0)
const souStatusText = computed(() => {
  if (mcpTools.value.length === 0)
    return '未知'
  return souEnabled.value ? '已启用' : '未启用'
})

// 获取输入组件的状态文本
const inputStatusText = computed(() => {
  return inputRef.value?.statusText || '等待输入...'
})

// UI/UX 上下文策略状态（用于可视化展示）
const policyStatus = computed(() => getContextPolicyStatus(props.request))
const showPolicyIndicator = computed(() => shouldShowPolicyIndicator(props.request))

// 加载继续回复配置
async function loadReplyConfig() {
  try {
    const config = await invoke('get_reply_config')
    if (config) {
      const replyConfig = config as any
      continueReplyEnabled.value = replyConfig.enable_continue_reply ?? true
      continuePrompt.value = replyConfig.continue_prompt ?? '请按照最佳实践继续'
    }
  }
  catch (error) {
    console.log('加载继续回复配置失败，使用默认值:', error)
  }
}

// 监听配置变化（当从设置页面切换回来时）
watch(() => props.appConfig.reply, (newReplyConfig) => {
  if (newReplyConfig) {
    continueReplyEnabled.value = newReplyConfig.enabled
    continuePrompt.value = newReplyConfig.prompt
  }
}, { deep: true, immediate: true })

// Telegram事件监听器
let telegramUnlisten: (() => void) | null = null

// 监听请求变化
watch(() => props.request, (newRequest) => {
  if (newRequest) {
    resetForm()
    loading.value = true
    // 每次显示弹窗时重新加载配置
    loadReplyConfig()

    // 如果有项目路径，启动索引状态轮询
    if (newRequest.project_root_path) {
      setCurrentProject(newRequest.project_root_path)
      startPolling(newRequest.project_root_path, 3000) // 3秒轮询间隔
    }
    else {
      // 没有项目路径时停止轮询
      stopPolling()
    }

    setTimeout(() => {
      loading.value = false
    }, 300)
  }
}, { immediate: true })

// 设置Telegram事件监听
async function setupTelegramListener() {
  try {
    telegramUnlisten = await listen('telegram-event', (event) => {
      console.log('🎯 [McpPopup] 收到Telegram事件:', event)
      console.log('🎯 [McpPopup] 事件payload:', event.payload)
      handleTelegramEvent(event.payload as any)
    })
    console.log('🎯 [McpPopup] Telegram事件监听器已设置')
  }
  catch (error) {
    console.error('🎯 [McpPopup] 设置Telegram事件监听器失败:', error)
  }
}

// 处理Telegram事件
function handleTelegramEvent(event: any) {
  console.log('🎯 [McpPopup] 开始处理事件:', event.type)

  switch (event.type) {
    case 'option_toggled':
      console.log('🎯 [McpPopup] 处理选项切换:', event.option)
      handleOptionToggle(event.option)
      break
    case 'text_updated':
      console.log('🎯 [McpPopup] 处理文本更新:', event.text)
      handleTextUpdate(event.text)
      break
    case 'continue_pressed':
      console.log('🎯 [McpPopup] 处理继续按钮')
      handleContinue()
      break
    case 'send_pressed':
      console.log('🎯 [McpPopup] 处理发送按钮')
      handleSubmit()
      break
    default:
      console.log('🎯 [McpPopup] 未知事件类型:', event.type)
  }
}

// 处理选项切换
function handleOptionToggle(option: string) {
  const index = selectedOptions.value.indexOf(option)
  if (index > -1) {
    // 取消选择
    selectedOptions.value.splice(index, 1)
  }
  else {
    // 添加选择
    selectedOptions.value.push(option)
  }

  // 同步到PopupInput组件
  if (inputRef.value) {
    inputRef.value.updateData({ selectedOptions: selectedOptions.value })
  }
}

// 处理文本更新
function handleTextUpdate(text: string) {
  userInput.value = text

  // 同步到PopupInput组件
  if (inputRef.value) {
    inputRef.value.updateData({ userInput: text })
  }
}

// 组件挂载时设置监听器和加载配置
onMounted(async () => {
  loadReplyConfig()
  setupTelegramListener()
  // 加载 MCP 工具配置（用于检测 sou 是否启用）
  await loadMcpTools()
  // 检测 ACE 配置是否完整
  acemcpConfigured.value = await checkAcemcpConfigured()
})

// 组件卸载时清理监听器
onUnmounted(() => {
  if (telegramUnlisten) {
    telegramUnlisten()
  }
  // 组件卸载时停止索引状态轮询
  stopPolling()
})

// 重置表单
function resetForm() {
  selectedOptions.value = []
  userInput.value = ''
  rawUserInput.value = ''
  conditionalContext.value = ''
  draggedImages.value = []
  submitting.value = false
}

// 构建用户回复摘要（不包含图片原始数据）
function buildUserReplySummary() {
  const parts: string[] = []
  const inputText = userInput.value.trim()
  if (inputText) {
    parts.push(`用户输入: ${inputText}`)
  }
  if (selectedOptions.value.length > 0) {
    parts.push(`选项: ${selectedOptions.value.join(', ')}`)
  }
  if (draggedImages.value.length > 0) {
    parts.push(`图片数量: ${draggedImages.value.length}`)
  }
  if (parts.length === 0) {
    parts.push('用户输入: 用户确认继续')
  }
  return parts.join('\n')
}

// 记录 zhi 历史（不影响主流程）
async function recordZhiHistory(userReplySummary = buildUserReplySummary()) {
  const projectRoot = props.request?.project_root_path
  if (!projectRoot) return

  const prompt = props.request?.message || ''
  const requestId = props.request?.id || ''

  try {
    await invoke('add_zhi_history', {
      projectRootPath: projectRoot,
      requestId,
      prompt,
      userReply: userReplySummary,
      source: 'popup',
    })
  }
  catch (error) {
    console.warn('记录 zhi 历史失败:', error)
  }
}

// 处理提交
async function handleSubmit() {
  if (!canSubmit.value || submitting.value)
    return

  submitting.value = true

  try {
    // 使用新的结构化数据格式
    const response = {
      user_input: userInput.value.trim() || null,
      selected_options: selectedOptions.value,
      images: draggedImages.value.map(imageData => ({
        data: imageData.split(',')[1], // 移除 data:image/png;base64, 前缀
        media_type: 'image/png',
        filename: null,
      })),
      metadata: {
        timestamp: new Date().toISOString(),
        request_id: props.request?.id || null,
        source: 'popup',
      },
    }

    // 如果没有任何有效内容，设置默认用户输入
    if (!response.user_input && response.selected_options.length === 0 && response.images.length === 0) {
      response.user_input = '用户确认继续'
    }

    if (props.mockMode) {
      // 模拟模式下的延迟
      await new Promise(resolve => setTimeout(resolve, 1000))
      message.success('模拟响应发送成功')
      // 模拟模式下也尝试记录历史（不阻塞）
      await recordZhiHistory()
    }
    else {
      // 实际发送响应
      await invoke('send_mcp_response', { response })
      // 发送成功后记录历史
      await recordZhiHistory()
      await invoke('exit_app')
    }

    emit('response', response)
  }
  catch (error) {
    console.error('提交响应失败:', error)
    message.error('提交失败，请重试')
  }
  finally {
    submitting.value = false
  }
}

// 处理输入更新
function handleInputUpdate(data: PopupInputUpdatePayload) {
  userInput.value = data.userInput
  rawUserInput.value = data.rawUserInput
  conditionalContext.value = data.conditionalContext
  selectedOptions.value = data.selectedOptions
  draggedImages.value = data.draggedImages
}

// 处理图片添加 - 移除重复逻辑，避免双重添加
function handleImageAdd(_image: string) {
  // 这个函数现在只是为了保持接口兼容性，实际添加在PopupInput中完成
}

// 处理图片移除
function handleImageRemove(index: number) {
  draggedImages.value.splice(index, 1)
}

// 处理继续按钮点击
async function handleContinue() {
  if (submitting.value)
    return

  submitting.value = true

  try {
    // 使用新的结构化数据格式
    const response = {
      user_input: continuePrompt.value,
      selected_options: [],
      images: [],
      metadata: {
        timestamp: new Date().toISOString(),
        request_id: props.request?.id || null,
        source: 'popup_continue',
      },
    }

    if (props.mockMode) {
      // 模拟模式下的延迟
      await new Promise(resolve => setTimeout(resolve, 1000))
      message.success('继续请求发送成功')
    }
    else {
      // 实际发送继续请求
      await invoke('send_mcp_response', { response })
      await invoke('exit_app')
    }

    emit('response', response)
  }
  catch (error) {
    console.error('发送继续请求失败:', error)
    message.error('继续请求失败，请重试')
  }
  finally {
    submitting.value = false
  }
}

// 处理引用消息
function handleQuoteMessage(messageContent: string) {
  if (inputRef.value) {
    inputRef.value.handleQuoteMessage(messageContent)
  }
}

// 处理增强按钮点击 - 直接走本地 AI 链路
async function handleEnhance() {
  if (submitting.value)
    return

  const rawInput = rawUserInput.value.trim()
  if (!rawInput) {
    message.warning('请先输入要增强的文本')
    return
  }

  submitting.value = true

  try {
    // 中文注释：复用发送按钮链路，把增强请求回送给本地 AI 会话
    const response = {
      user_input: buildLocalEnhancePrompt(rawInput),
      selected_options: [],
      images: [],
      metadata: {
        timestamp: new Date().toISOString(),
        request_id: props.request?.id || null,
        source: 'popup_local_enhance',
      },
    }

    if (props.mockMode) {
      await new Promise(resolve => setTimeout(resolve, 1000))
      message.success('本地增强请求发送成功')
      await recordZhiHistory(buildLocalEnhanceHistorySummary(rawInput))
    }
    else {
      await invoke('send_mcp_response', { response })
      await recordZhiHistory(buildLocalEnhanceHistorySummary(rawInput))
      await invoke('exit_app')
    }

    emit('response', response)
  }
  catch (error) {
    console.error('发送本地增强请求失败:', error)
    message.error('本地增强请求失败，请重试')
  }
  finally {
    submitting.value = false
  }
}

// 处理跳转 MCP 工具页
function handleOpenMcpToolsTab(toolId?: string) {
  emit('openMcpToolsTab', toolId)
}

// 实际执行索引同步/重建
async function runIndexResync(type: 'incremental' | 'full') {
  if (!props.request?.project_root_path || resyncLoading.value)
    return

  resyncLoading.value = true
  try {
    const result = await triggerIndexUpdate(props.request.project_root_path, type)
    const fallback = type === 'full' ? '全量重建已触发' : '增量同步已触发'
    const messageText = typeof result === 'string' ? result : fallback
    message.success(type === 'full' ? `全量重建：${messageText}` : messageText)
  }
  catch (error) {
    console.error('触发索引更新失败:', error)
    message.error(`触发索引更新失败: ${String(error)}`)
  }
  finally {
    resyncLoading.value = false
  }
}

function getCodeAnalysisInstruction() {
  switch (souStatusText.value) {
    case '已启用':
      return '必须先使用 sou 语义搜索定位真实实现，再结合命中文件完成代码确认。'
    case '未启用':
      return 'sou 当前未启用，必须改用项目内代码搜索（如 rg）和真实文件阅读完成代码分析。'
    default:
      return '优先尝试 sou；若不可用，则必须改用项目内代码搜索（如 rg）和真实文件阅读完成代码分析。'
  }
}

function buildLocalEnhancePrompt(rawInput: string) {
  const projectRoot = props.request?.project_root_path?.trim() || '未提供'
  const background = conditionalContext.value.trim()
  const sections = [
    '# 本地提示词增强请求',
    '这是一次“本地提示词增强”操作，不是对当前弹窗问题的直接答复。',
    '你的唯一任务：基于真实代码分析，将“原始口语化输入”改写为结构化、清晰、可执行的高质量提示词。',
    '## 强制规则',
    '1. 必须先做真实代码分析，禁止凭空补全或脑补实现。',
    `2. ${getCodeAnalysisInstruction()}`,
    '3. 如果没有项目路径、无法检索真实代码、或检索结果不足以支撑结论，必须明确返回失败，不要静默降级。',
    '4. 只允许改写“原始口语化输入”；附加背景仅供理解，不能直接并入最终增强正文。',
    '5. 输出语言以中文为主。',
    '6. 不要解释分析过程，不要回答原弹窗问题，不要输出多余寒暄。',
    '## 成功输出格式（必须严格遵守）',
    '### BEGIN RESPONSE ###',
    '<augment-enhanced-prompt>这里放增强后的结构化提示词</augment-enhanced-prompt>',
    '### END RESPONSE ###',
    '## 无法完成时的输出格式（必须严格遵守）',
    '### BEGIN RESPONSE ###',
    '无法基于真实代码分析生成增强提示词：请写明具体原因',
    '### END RESPONSE ###',
    '## 项目分析条件',
    `- 项目根路径：${projectRoot}`,
    `- sou 工具前端检测状态：${souStatusText.value}`,
  ]

  if (background) {
    sections.push(
      '## 附加背景（仅供理解，不得直接复制到最终结果）',
      `<additional-background>\n${background}\n</additional-background>`,
    )
  }

  sections.push(
    '## 原始口语化输入（这是唯一需要被改写的正文）',
    `<original-user-input>\n${rawInput}\n</original-user-input>`,
  )

  return sections.join('\n\n')
}

function buildLocalEnhanceHistorySummary(rawInput: string) {
  const parts = [`本地增强请求: ${rawInput}`]
  if (conditionalContext.value.trim()) {
    parts.push('附加背景: 已附带条件性上下文')
  }
  return parts.join('\n')
}

// 处理索引重新同步请求
function handleIndexResync(type: 'incremental' | 'full') {
  if (!props.request?.project_root_path || resyncLoading.value)
    return

  if (type === 'full') {
    const projectRoot = props.request.project_root_path
    dialog.warning({
      title: '确认全量重建',
      content: `将清理本地索引记录并重新上传所有文件。\n\n项目：${projectRoot}\n\n过程较慢，但不会阻塞当前对话。是否继续？`,
      positiveText: '继续',
      negativeText: '取消',
      onPositiveClick: async () => {
        await runIndexResync(type)
      },
    })
    return
  }

  runIndexResync(type)
}

// 处理打开索引详情抽屉
function handleOpenIndexStatus() {
  emit('openIndexStatus')
}
</script>

<template>
  <div v-if="isVisible" class="flex flex-col flex-1 min-h-0 overflow-hidden">
    <!-- ACE 索引状态面板（智能降级：根据 sou 启用状态和 ACE 配置显示不同内容） -->
    <ZhiIndexPanel
      :project-root="request?.project_root_path"
      :sou-enabled="souEnabled"
      :acemcp-configured="acemcpConfigured"
      :project-status="currentProjectStatus"
      :is-indexing="isIndexing"
      :resync-loading="resyncLoading"
      @open-settings="handleOpenMcpToolsTab"
      @open-detail="handleOpenIndexStatus"
      @resync="handleIndexResync"
    />

    <!-- UI/UX 上下文策略指示器（全局提示，便于统一感知） -->
    <div
      v-if="showPolicyIndicator"
      class="mx-2 mt-2 px-3 py-2.5 bg-black-100/90 rounded-xl border border-gray-700/60"
    >
      <n-tooltip trigger="hover" placement="bottom">
        <template #trigger>
          <div class="flex flex-col gap-1.5 text-xs cursor-help">
            <div class="flex items-center gap-2">
              <div :class="[policyStatus.icon, policyStatus.colorClass]" class="w-4 h-4" />
              <span class="text-white/80">UI/UX 追加：</span>
              <span :class="policyStatus.colorClass" class="font-medium">{{ policyStatus.label }}</span>
            </div>
            <!-- 全局提示时始终展示原因，避免默认策略被误解 -->
            <div
              class="text-[11px] leading-4"
              :class="policyStatus.allowed ? 'text-white/65' : 'text-yellow-200/80'"
            >
              {{ policyStatus.reason }}
            </div>
          </div>
        </template>
        <div class="text-xs space-y-1 max-w-[280px]">
          <div class="font-medium">UI/UX 上下文策略详情</div>
          <div>{{ policyStatus.reason }}</div>
          <div class="text-white/60 pt-1 border-t border-white/10">
            意图：{{ policyStatus.intent }} · 策略：{{ policyStatus.policy }}
          </div>
        </div>
      </n-tooltip>
    </div>

    <!-- 内容区域 - 可滚动 -->
    <div class="flex-1 min-h-0 overflow-y-auto overflow-x-hidden">
      <!-- 消息内容 - 允许选中 -->
      <div class="mx-2 mt-2 mb-1 px-4 py-3 bg-black-100 rounded-lg select-text" data-guide="popup-content">
        <PopupContent :request="request" :loading="loading" :current-theme="props.appConfig.theme" @quote-message="handleQuoteMessage" />
      </div>

      <!-- 输入和选项 - 允许选中 -->
      <div class="px-4 pb-3 bg-black select-text">
        <PopupInput
          ref="inputRef" :request="request" :loading="loading" :submitting="submitting"
          :enhance-enabled="localEnhanceEnabled"
          @update="handleInputUpdate" @image-add="handleImageAdd" @image-remove="handleImageRemove"
          @enhance="handleEnhance"
          @open-mcp-tools-tab="handleOpenMcpToolsTab"
        />
      </div>
    </div>

    <!-- 底部操作栏 - 固定在底部 -->
    <div class="flex-shrink-0 bg-black-100 border-t-2 border-black-200" data-guide="popup-actions">
      <PopupActions
        :request="request" :loading="loading" :submitting="submitting" :can-submit="canSubmit"
        :can-enhance="canEnhance"
        :continue-reply-enabled="continueReplyEnabled" :input-status-text="inputStatusText"
        :enhance-enabled="localEnhanceEnabled"
        @submit="handleSubmit" @continue="handleContinue" @enhance="handleEnhance"
        @open-mcp-tools-tab="handleOpenMcpToolsTab"
      />
    </div>
  </div>
</template>
