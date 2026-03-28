<script setup lang="ts">
import { invoke } from '@tauri-apps/api/core'
import { useMessage } from 'naive-ui'
import { computed, ref } from 'vue'
import IntroTab from '../tabs/IntroTab.vue'
import McpToolsTab from '../tabs/McpToolsTab.vue'
import PromptsTab from '../tabs/PromptsTab.vue'
import SettingsTab from '../tabs/SettingsTab.vue'

interface Props {
  currentTheme: string
  alwaysOnTop: boolean
  audioNotificationEnabled: boolean
  audioUrl: string
  windowWidth: number
  windowHeight: number
  fixedWindowSize: boolean
  activeTab?: string
  projectRootPath?: string
  autoOpenToolId?: string
  autoOpenToolRequestId?: number
}

interface Emits {
  themeChange: [theme: string]
  toggleAlwaysOnTop: []
  toggleAudioNotification: []
  updateAudioUrl: [url: string]
  testAudio: []
  stopAudio: []
  testAudioError: [error: any]
  updateWindowSize: [size: { width: number, height: number, fixed: boolean }]
  configReloaded: []
  'update:activeTab': [tab: string]
  mcpToolAutoOpened: [requestId: number]
}

const props = defineProps<Props>()
const emit = defineEmits<Emits>()

// 处理配置重新加载事件
function handleConfigReloaded() {
  emit('configReloaded')
}

const internalActiveTab = ref('intro')
// 支持外部受控切换 Tab，未传入时使用内部状态
const activeTab = computed({
  get: () => props.activeTab ?? internalActiveTab.value,
  set: (value) => {
    internalActiveTab.value = value
    emit('update:activeTab', value)
  },
})
const message = useMessage()

// 图标加载错误处理
function handleImageError(event: Event) {
  const img = event.target as HTMLImageElement
  // 如果图标加载失败，隐藏图片元素
  img.style.display = 'none'
  console.warn('LOGO图标加载失败，已隐藏')
}

// 测试popup功能 - 创建独立的popup窗口
async function showTestMcpPopup() {
  try {
    // 创建测试请求数据
    const testRequest = {
      id: `test-${Date.now()}`,
      message: `# 🧪 测试弹窗功能

这是一个**测试弹窗**，用于验证MCP popup组件的功能。

## 功能特性
- ✅ 支持 Markdown 格式显示
- ✅ 支持预定义选项选择
- ✅ 支持自由文本输入
- ✅ 支持图片粘贴上传

## 代码示例
\`\`\`javascript
// 这是一个代码示例
function testPopup() {
  console.log('测试弹窗功能')
  return '成功'
}
\`\`\`

请选择您要测试的功能，或者在下方输入框中添加您的反馈。`,
      predefined_options: ['测试选项功能', '测试文本输入', '测试图片上传', '测试Markdown渲染'],
      is_markdown: true,
    }

    // 调用Tauri命令创建popup窗口
    await invoke('create_test_popup', { request: testRequest })
    message.success('测试popup窗口已创建')
  }
  catch (error) {
    console.error('创建测试popup失败:', error)
    message.error(`创建测试popup失败: ${error}`)
  }
}
</script>

<template>
  <div class="flex flex-col min-h-screen">
    <!-- 主要内容区域 -->
    <div class="flex-1 flex items-start justify-center p-6 pt-8">
      <div class="max-w-6xl w-full">
        <!-- 标题区域 -->
        <div class="text-center mb-8">
          <!-- 主标题 -->
          <div class="flex items-center justify-center gap-3 mb-3" data-guide="app-logo">
            <img
              src="/icons/icon-128.png"
              alt="三术 Logo"
              class="w-10 h-10 rounded-xl shadow-lg"
              @error="handleImageError"
            >
            <h1 class="text-4xl font-medium text-on-surface">
              三术
            </h1>
            <!-- 测试按钮 -->
            <n-button
              size="small"
              type="tertiary"
              circle
              title="测试 Popup 功能"
              class="ml-2"
              data-guide="test-button"
              @click="showTestMcpPopup"
            >
              <template #icon>
                <div class="i-carbon-test-tool w-4 h-4" />
              </template>
            </n-button>
          </div>

          <!-- 服务器状态 -->
          <div class="mb-4">
            <n-tag type="success" size="small" round class="px-3 py-1">
              <template #icon>
                <div class="w-2 h-2 bg-success rounded-full animate-pulse" />
              </template>
              MCP 服务已启动
            </n-tag>
          </div>

          <!-- 副标题 -->
          <p class="text-base opacity-50 font-normal text-on-surface">
            道生一，一生二，二生三，三生万物
          </p>
        </div>

        <!-- Tab组件 -->
        <n-tabs v-model:value="activeTab" type="segment" size="small" justify-content="center" data-guide="tabs">
          <n-tab-pane name="intro" tab="介绍">
            <IntroTab />
          </n-tab-pane>
          <n-tab-pane name="mcp-tools" tab="MCP 工具">
            <McpToolsTab
              :project-root-path="projectRootPath"
              :auto-open-tool-id="props.autoOpenToolId"
              :auto-open-tool-request-id="props.autoOpenToolRequestId"
              @auto-open-handled="$emit('mcpToolAutoOpened', $event)"
            />
          </n-tab-pane>
          <n-tab-pane name="prompts" tab="参考提示词">
            <PromptsTab />
          </n-tab-pane>
          <n-tab-pane name="settings" tab="设置" data-guide="settings-tab">
            <SettingsTab
              :current-theme="currentTheme"
              :always-on-top="alwaysOnTop"
              :audio-notification-enabled="audioNotificationEnabled"
              :audio-url="audioUrl"
              :window-width="windowWidth"
              :window-height="windowHeight"
              :fixed-window-size="fixedWindowSize"
              @theme-change="$emit('themeChange', $event)"
              @toggle-always-on-top="$emit('toggleAlwaysOnTop')"
              @toggle-audio-notification="$emit('toggleAudioNotification')"
              @update-audio-url="$emit('updateAudioUrl', $event)"
              @test-audio="$emit('testAudio')"
              @stop-audio="$emit('stopAudio')"
              @test-audio-error="$emit('testAudioError', $event)"
              @update-window-size="$emit('updateWindowSize', $event)"
              @config-reloaded="handleConfigReloaded"
            />
          </n-tab-pane>
        </n-tabs>
      </div>
    </div>
  </div>
</template>
