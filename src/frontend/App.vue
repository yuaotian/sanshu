<script setup lang="ts">
import { onMounted, onUnmounted, watch } from 'vue'
import AppContent from './components/AppContent.vue'
import { useAppManager } from './composables/useAppManager'
import { useEventHandlers } from './composables/useEventHandlers'

// 使用封装的应用管理器
const {
  naiveTheme,
  mcpRequest,
  showMcpPopup,
  appConfig,
  isInitializing,
  isIconMode,
  iconParams,
  actions,
} = useAppManager()

// 创建事件处理器
const handlers = useEventHandlers(actions)

// 主题应用由useTheme统一管理，移除重复的主题应用逻辑

function setRootScrollLock(locked: boolean) {
  document.documentElement.style.overflow = locked ? 'hidden' : ''
  document.body.style.overflow = locked ? 'hidden' : ''
}

// 仅在三术弹窗模式锁定根滚动，避免双纵向滚动条
watch(showMcpPopup, (v) => {
  setRootScrollLock(!!v)
}, { immediate: true })

// 初始化
onMounted(async () => {
  try {
    await actions.app.initialize()
  }
  catch (error) {
    console.error('应用初始化失败:', error)
  }
})

// 清理
onUnmounted(() => {
  setRootScrollLock(false)
  actions.app.cleanup()
})
</script>

<template>
  <div :class="showMcpPopup ? 'h-screen overflow-hidden bg-surface transition-colors duration-200' : 'min-h-screen bg-surface transition-colors duration-200'">
    <n-config-provider :theme="naiveTheme">
      <n-message-provider>
        <n-notification-provider>
          <n-dialog-provider>
            <AppContent
              :mcp-request="mcpRequest" 
              :show-mcp-popup="showMcpPopup" 
              :app-config="appConfig"
              :is-initializing="isInitializing" 
              :is-icon-mode="isIconMode"
              :icon-params="iconParams"
              @mcp-response="handlers.onMcpResponse" 
              @mcp-cancel="handlers.onMcpCancel"
              @theme-change="handlers.onThemeChange" 
              @toggle-always-on-top="handlers.onToggleAlwaysOnTop"
              @toggle-audio-notification="handlers.onToggleAudioNotification"
              @update-audio-url="handlers.onUpdateAudioUrl" 
              @test-audio="handlers.onTestAudio"
              @stop-audio="handlers.onStopAudio" 
              @test-audio-error="handlers.onTestAudioError"
              @update-window-size="handlers.onUpdateWindowSize"
              @update-reply-config="handlers.onUpdateReplyConfig" 
              @message-ready="handlers.onMessageReady"
              @config-reloaded="handlers.onConfigReloaded"
            />
          </n-dialog-provider>
        </n-notification-provider>
      </n-message-provider>
    </n-config-provider>
  </div>
</template>

