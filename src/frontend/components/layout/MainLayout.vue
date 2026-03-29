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
      message: `# 一级标题

这是一段**加粗文本**和*斜体文本*以及~~删除线~~。行内代码：\`console.log('hello')\`。

## 二级标题

### 三级标题

#### 四级标题

---

## 列表

无序列表：
- 第一项
- 第二项
  - 嵌套项 A
  - 嵌套项 B
- 第三项

有序列表：
1. 步骤一
2. 步骤二
3. 步骤三

## 引用

> 这是一段引用文本，用来测试引用块的样式。
> 第二行引用。

## 表格

| 属性 | 类型 | 默认值 | 说明 |
|------|------|--------|------|
| size | string | medium | 组件尺寸 |
| disabled | boolean | false | 是否禁用 |
| loading | boolean | false | 加载状态 |

## 代码块

\`\`\`typescript
interface UserConfig {
  theme: 'light' | 'dark'
  lang: string
}

function loadConfig(): UserConfig {
  return { theme: 'dark', lang: 'zh-CN' }
}
\`\`\`

横向滚动测试：

\`\`\`javascript
const veryLongVariableName = { key1: 'value1', key2: 'value2', key3: 'value3', key4: 'value4', key5: 'value5', key6: 'value6', key7: 'value7', key8: 'value8', key9: 'value9', key10: 'value10' }
\`\`\`

## 链接

这是一个 [GitHub](https://github.com) 链接，点击后应在外部浏览器打开。

普通段落文本用于测试行距和字号是否符合 Naive UI 规范。`,
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
