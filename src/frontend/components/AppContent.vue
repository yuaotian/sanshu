<script setup lang="ts">
import { useElementSize } from '@vueuse/core'
import { useMessage } from 'naive-ui'
import { computed, onMounted, onUnmounted, ref, watch } from 'vue'
import { useAcemcpSync } from '../composables/useAcemcpSync'
import { setupExitWarningListener } from '../composables/useExitWarning'
import { useKeyboard } from '../composables/useKeyboard'
import { useLogViewer } from '../composables/useLogViewer'
import { useMcpToolsReactive } from '../composables/useMcpTools'
import WindowTitleBar from './common/WindowTitleBar.vue'
import LayoutWrapper from './layout/LayoutWrapper.vue'
import McpIndexStatusDrawer from './popup/McpIndexStatusDrawer.vue'
import McpPopup from './popup/McpPopup.vue'
import PopupHeader from './popup/PopupHeader.vue'
import AcemcpLogViewerDrawer from './tools/AcemcpLogViewerDrawer.vue'
import IconPopupMode from './tools/IconWorkshop/IconPopupMode.vue'

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
  mcpRequest: any
  showMcpPopup: boolean
  appConfig: AppConfig
  isInitializing: boolean
  isIconMode?: boolean
  iconParams?: {
    query: string
    style: string
    savePath: string
    projectRoot: string
  } | null
}

interface Emits {
  mcpResponse: [response: any]
  mcpCancel: []
  themeChange: [theme: string]
  toggleAlwaysOnTop: []
  toggleAudioNotification: []
  updateAudioUrl: [url: string]
  testAudio: []
  stopAudio: []
  testAudioError: [error: any]
  updateWindowSize: [size: { width: number, height: number, fixed: boolean }]
  updateReplyConfig: [config: { enable_continue_reply?: boolean, continue_prompt?: string }]
  messageReady: [message: any]
  configReloaded: []
}

const props = defineProps<Props>()
const emit = defineEmits<Emits>()

// 标题栏 fixed 定位后的高度占位
const titleBarRef = ref<HTMLElement | null>(null)
const { height: titleBarHeight } = useElementSize(titleBarRef)

// 将标题栏高度同步到 CSS 变量，供全局样式（toast/modal 偏移）使用
watch(titleBarHeight, (h) => {
  document.documentElement.style.setProperty('--title-bar-height', `${h}px`)
}, { immediate: true })

// 弹窗中的设置显示控制
const showPopupSettings = ref(false)
// 设置界面当前激活的 Tab
const activeTab = ref('intro')
// 记录待自动打开的 MCP 工具配置，避免用户还要再点一次“配置”。
const pendingMcpToolConfig = ref<{ toolId: string, requestId: number } | null>(null)
// MCP 索引详情抽屉显示控制
const showIndexDrawer = ref(false)
// 全局日志查看器显示控制（用于“整个弹窗”的日志查看）
const { show: showLogViewer, open: openLogViewer } = useLogViewer()

// 初始化 Naive UI 消息实例
const message = useMessage()

// 键盘快捷键处理
const { handleExitShortcut } = useKeyboard()

// MCP 工具与索引状态
const {
  mcpTools,
  loadMcpTools,
} = useMcpToolsReactive()

const {
  currentProjectStatus,
  statusSummary,
  statusIcon,
  isIndexing,
  triggerIndexUpdate,
} = useAcemcpSync()

// 记录重新同步按钮的本地 loading 状态
const resyncLoading = ref(false)

// 是否启用 sou 代码搜索工具
const souEnabled = computed(() => mcpTools.value.some(tool => tool.id === 'sou' && tool.enabled))
// 是否启用提示词增强工具
const enhanceEnabled = computed(() => mcpTools.value.some(tool => tool.id === 'enhance' && tool.enabled))

// Header 中是否需要展示 MCP 索引状态指示器
const showMcpIndexStatus = computed(() => {
  return souEnabled.value
    && !!props.mcpRequest?.project_root_path
    && !!currentProjectStatus.value
})

// 从项目路径提取项目名（取最后一级目录名）
const projectName = computed(() => {
  const p = props.mcpRequest?.project_root_path
  if (!p) return ''
  const normalized = p.replace(/\\/g, '/')
  const segments = normalized.split('/').filter(Boolean)
  return segments[segments.length - 1] || ''
})

// Header Tooltip 使用的错误与告警摘要信息
const mcpFailedFiles = computed(() => currentProjectStatus.value?.failed_files ?? 0)
const mcpLastFailureTime = computed(() => currentProjectStatus.value?.last_failure_time || null)
const mcpLastError = computed(() => currentProjectStatus.value?.last_error || null)

// 切换弹窗设置显示
function togglePopupSettings() {
  showPopupSettings.value = !showPopupSettings.value
}

// 直接打开 MCP 工具页（用于 CTA 跳转）
function openMcpToolsTab(toolId?: string) {
  activeTab.value = 'mcp-tools'
  showPopupSettings.value = true
  if (toolId) {
    pendingMcpToolConfig.value = {
      toolId,
      requestId: Date.now(),
    }
  }
}

function handleMcpToolAutoOpened(requestId: number) {
  if (pendingMcpToolConfig.value?.requestId === requestId)
    pendingMcpToolConfig.value = null
}

// 处理索引详情抽屉中的重新同步请求
async function handleIndexResync() {
  if (!props.mcpRequest?.project_root_path || resyncLoading.value)
    return

  resyncLoading.value = true
  try {
    // 调用索引更新命令，并依赖 useAcemcpSync 轮询结果刷新状态
    const result = await triggerIndexUpdate(props.mcpRequest.project_root_path)
    message.success(typeof result === 'string' ? result : '索引更新已触发')
  }
  catch (error) {
    console.error('触发索引更新失败:', error)
    message.error(`触发索引更新失败: ${String(error)}`)
  }
  finally {
    resyncLoading.value = false
  }
}

// 监听 MCP 请求变化，当有新请求时重置设置页面状态
watch(() => props.mcpRequest, (newRequest) => {
  if (newRequest && showPopupSettings.value) {
    // 有新的 MCP 请求时，自动切换回消息页面
    showPopupSettings.value = false
    activeTab.value = 'intro'
    pendingMcpToolConfig.value = null
  }
}, { immediate: true })

// 全局键盘事件处理器
function handleGlobalKeydown(event: KeyboardEvent) {
  handleExitShortcut(event)
}

onMounted(() => {
  // 将消息实例传递给父组件
  emit('messageReady', message)
  // 设置退出警告监听器（统一处理主界面和弹窗）
  setupExitWarningListener(message)

  // 添加全局键盘事件监听器
  document.addEventListener('keydown', handleGlobalKeydown)

  // 加载 MCP 工具配置（用于检测 sou 是否启用）
  loadMcpTools()
})

onUnmounted(() => {
  // 移除键盘事件监听器
  document.removeEventListener('keydown', handleGlobalKeydown)
})
</script>

<template>
  <div :class="props.showMcpPopup ? 'h-screen overflow-hidden bg-surface' : 'min-h-screen bg-surface'">
    <!-- 图标搜索弹窗模式 -->
    <IconPopupMode
      v-if="props.isIconMode && props.iconParams"
      :initial-query="props.iconParams.query"
      :initial-style="props.iconParams.style"
      :initial-save-path="props.iconParams.savePath"
      :project-root="props.iconParams.projectRoot"
    />

    <!-- MCP弹窗模式 -->
    <div
      v-else-if="props.showMcpPopup && props.mcpRequest"
      class="flex flex-col w-full h-screen bg-surface text-on-surface select-none overflow-hidden"
    >
      <!-- 头部 - 固定在顶部，z-index 高于 Naive UI modal 遮罩层以保证拖拽可用 -->
      <div ref="titleBarRef" class="fixed top-0 left-0 right-0 z-[9999] bg-container-secondary border-b-2 border-border">
        <PopupHeader
          :current-theme="props.appConfig.theme"
          :loading="false"
          :show-main-layout="showPopupSettings"
          :always-on-top="props.appConfig.window.alwaysOnTop"
          :project-name="projectName"
          :mcp-enabled="showMcpIndexStatus"
          :mcp-status-summary="statusSummary"
          :mcp-status-icon="statusIcon"
          :mcp-is-indexing="isIndexing"
          :mcp-failed-files="mcpFailedFiles"
          :mcp-last-failure-time="mcpLastFailureTime"
          :mcp-last-error="mcpLastError"
          @theme-change="$emit('themeChange', $event)"
          @open-main-layout="togglePopupSettings"
          @open-log-viewer="openLogViewer"
          @toggle-always-on-top="$emit('toggleAlwaysOnTop')"
          @open-index-status="showIndexDrawer = true"
        />
      </div>
      <!-- 标题栏占位，补偿 fixed 脱离文档流的高度 -->
      <div :style="{ height: titleBarHeight + 'px' }" class="flex-shrink-0" />

      <!-- 设置界面 -->
      <div
        v-if="showPopupSettings"
        class="flex-1 overflow-y-auto scrollbar-thin"
      >
        <LayoutWrapper
          :app-config="props.appConfig"
          :active-tab="activeTab"
          :project-root-path="props.mcpRequest?.project_root_path || null"
          :auto-open-tool-id="pendingMcpToolConfig?.toolId || null"
          :auto-open-tool-request-id="pendingMcpToolConfig?.requestId || 0"
          @theme-change="$emit('themeChange', $event)"
          @toggle-always-on-top="$emit('toggleAlwaysOnTop')"
          @toggle-audio-notification="$emit('toggleAudioNotification')"
          @update-audio-url="$emit('updateAudioUrl', $event)"
          @test-audio="$emit('testAudio')"
          @stop-audio="$emit('stopAudio')"
          @test-audio-error="$emit('testAudioError', $event)"
          @update-window-size="$emit('updateWindowSize', $event)"
          @update:active-tab="activeTab = $event"
          @mcp-tool-auto-opened="handleMcpToolAutoOpened"
        />
      </div>

      <!-- 弹窗内容 -->
      <McpPopup
        v-else
        class="flex-1 min-h-0"
        :request="props.mcpRequest"
        :app-config="props.appConfig"
        :enhance-enabled="enhanceEnabled"
        @response="$emit('mcpResponse', $event)"
        @cancel="$emit('mcpCancel')"
        @theme-change="$emit('themeChange', $event)"
        @open-mcp-tools-tab="openMcpToolsTab"
        @open-index-status="showIndexDrawer = true"
      />

      <!-- MCP 代码索引详情抽屉 -->
      <McpIndexStatusDrawer
        v-if="props.mcpRequest?.project_root_path"
        :show="showIndexDrawer"
        :project-root="props.mcpRequest.project_root_path"
        :status-summary="statusSummary"
        :status-icon="statusIcon"
        :project-status="currentProjectStatus"
        :is-indexing="isIndexing"
        :resync-loading="resyncLoading"
        @update:show="showIndexDrawer = $event"
        @resync="handleIndexResync"
      />
    </div>

    <!-- 弹窗加载骨架屏 或 初始化骨架屏 -->
    <div
      v-else-if="props.showMcpPopup || props.isInitializing"
      class="flex flex-col w-full h-screen bg-surface text-on-surface"
    >
      <!-- 头部 -->
      <div class="flex-shrink-0 bg-container-secondary border-b-2 border-border">
        <WindowTitleBar title="三术 - 加载中..." />
      </div>

      <!-- 内容骨架 -->
      <div class="flex-1 p-4">
        <div class="bg-container rounded-lg p-4 mb-4">
          <n-skeleton
            text
            :repeat="3"
          />
        </div>

        <div class="space-y-3">
          <n-skeleton
            text
            :width="128"
          />
          <n-skeleton
            text
            :repeat="3"
          />
        </div>
      </div>

      <!-- 底部骨架 -->
      <div class="flex-shrink-0 bg-container-secondary border-t-2 border-border p-4">
        <div class="flex justify-between items-center">
          <n-skeleton
            text
            :width="96"
          />
          <div class="flex gap-2">
            <n-skeleton
              text
              :width="64"
              :height="32"
            />
            <n-skeleton
              text
              :width="64"
              :height="32"
            />
          </div>
        </div>
      </div>
    </div>

    <!-- 主界面 - 只在非弹窗模式且非初始化时显示 -->
    <div v-else class="flex flex-col h-screen overflow-hidden">
      <div class="flex-shrink-0 bg-container-secondary border-b border-border">
        <WindowTitleBar title="三术 - 设置" :current-theme="props.appConfig.theme" @theme-change="$emit('themeChange', $event)" />
      </div>
      <div class="flex-1 min-h-0 overflow-y-auto">
      <LayoutWrapper
        :app-config="props.appConfig"
        :active-tab="activeTab"
        :project-root-path="null"
      :auto-open-tool-id="pendingMcpToolConfig?.toolId || null"
      :auto-open-tool-request-id="pendingMcpToolConfig?.requestId || 0"
      @theme-change="$emit('themeChange', $event)"
      @toggle-always-on-top="$emit('toggleAlwaysOnTop')"
      @toggle-audio-notification="$emit('toggleAudioNotification')"
      @update-audio-url="$emit('updateAudioUrl', $event)"
      @test-audio="$emit('testAudio')"
      @stop-audio="$emit('stopAudio')"
      @test-audio-error="$emit('testAudioError', $event)"
      @update-window-size="$emit('updateWindowSize', $event)"
      @config-reloaded="$emit('configReloaded')"
      @update:active-tab="activeTab = $event"
      @mcp-tool-auto-opened="handleMcpToolAutoOpened"
    />
      </div>
    </div>

    <!-- 全局日志查看器抽屉：主界面/弹窗模式均可打开 -->
    <AcemcpLogViewerDrawer v-model:show="showLogViewer" />
  </div>
</template>
