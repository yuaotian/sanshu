<script setup lang="ts">
import { invoke } from '@tauri-apps/api/core'
import { listen } from '@tauri-apps/api/event'
import { useMessage } from 'naive-ui'
import { onMounted, onUnmounted, ref } from 'vue'
import AudioSettings from '../settings/AudioSettings.vue'
import CustomPromptSettings from '../settings/CustomPromptSettings.vue'
import FontSettings from '../settings/FontSettings.vue'
import ProjectIndexManager from '../settings/ProjectIndexManager.vue'
import ProxySettings from '../settings/ProxySettings.vue'
import ReplySettings from '../settings/ReplySettings.vue'
import ShortcutSettings from '../settings/ShortcutSettings.vue'
import TelegramSettings from '../settings/TelegramSettings.vue'
import WindowSettings from '../settings/WindowSettings.vue'

const expandedCards = ref<string[]>([])
const imageCompressionEnabled = ref(true)

async function loadImageCompressionSetting() {
  try {
    imageCompressionEnabled.value = await invoke('get_image_compression_enabled') as boolean
  }
  catch {
    imageCompressionEnabled.value = true
  }
}

async function toggleImageCompression(val: boolean) {
  try {
    await invoke('set_image_compression_enabled', { enabled: val })
    imageCompressionEnabled.value = val
  }
  catch (error) {
    message.error(`设置图片压缩失败: ${error}`)
  }
}

function toggleCard(name: string) {
  const idx = expandedCards.value.indexOf(name)
  if (idx >= 0) {
    expandedCards.value.splice(idx, 1)
  }
  else {
    expandedCards.value.push(name)
  }
}

function isExpanded(name: string) {
  return expandedCards.value.indexOf(name) >= 0
}

interface Props {
  currentTheme: string
  alwaysOnTop: boolean
  audioNotificationEnabled: boolean
  audioUrl: string
  windowWidth: number
  windowHeight: number
  fixedWindowSize: boolean
}

defineProps<Props>()
const emit = defineEmits<Emits>()
const message = useMessage()
const isReloading = ref(false)
const configFilePath = ref('config.json')
let unlistenConfigReloaded: (() => void) | null = null

// 重新加载配置（通过重新加载设置实现）
async function reloadConfig() {
  if (isReloading.value)
    return

  isReloading.value = true
  try {
    // 触发重新加载设置的事件
    emit('configReloaded')
    message.success('配置已重新加载')
  }
  catch (error) {
    console.error('重新加载配置失败:', error)
    message.error('重新加载配置失败')
  }
  finally {
    isReloading.value = false
  }
}

// 获取配置文件路径
async function loadConfigFilePath() {
  try {
    const path = await invoke('get_config_file_path')
    configFilePath.value = path as string
    console.log('配置文件路径:', configFilePath.value)
  }
  catch (error) {
    console.error('获取配置文件路径失败:', error)
    configFilePath.value = 'config.json' // 使用默认值
  }
}

// 监听配置重载事件
onMounted(async () => {
  try {
    await loadImageCompressionSetting()
    await loadConfigFilePath()

    unlistenConfigReloaded = await listen('config_reloaded', () => {
      // 配置重载后，重新加载设置而不是刷新整个页面
      console.log('收到配置重载事件，重新加载设置')
      // 触发重新加载设置的事件
      emit('configReloaded')
    })
  }
  catch (error) {
    console.error('设置配置重载监听器失败:', error)
  }
})

onUnmounted(() => {
  if (unlistenConfigReloaded) {
    unlistenConfigReloaded()
  }
})

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
}

// 处理窗口尺寸更新
function handleWindowSizeUpdate(size: { width: number, height: number, fixed: boolean }) {
  emit('updateWindowSize', size)
}
</script>

<template>
  <div class="max-w-3xl mx-auto space-y-3">
    <!-- 字体设置 -->
    <n-card size="small" hoverable>
      <template #header>
        <div class="flex items-center cursor-pointer" @click="toggleCard('font')">
          <div class="w-10 h-10 rounded-lg bg-orange-100 dark:bg-orange-900 flex items-center justify-center mr-4">
            <div class="i-carbon-text-font text-lg text-orange-600 dark:text-orange-400" />
          </div>
          <div class="flex-1">
            <div class="text-base font-medium tracking-tight">
              字体设置
            </div>
            <div class="text-xs opacity-60 font-normal mt-0.5">
              自定义应用字体系列和大小
            </div>
          </div>
          <div class="i-carbon-chevron-down text-lg opacity-40 transition-transform duration-200" :class="{ 'rotate-180': isExpanded('font') }" />
        </div>
      </template>
      <n-collapse-transition :show="isExpanded('font')">
        <FontSettings />
      </n-collapse-transition>
    </n-card>

    <!-- 继续回复设置 -->
    <n-card size="small" hoverable>
      <template #header>
        <div class="flex items-center cursor-pointer" @click="toggleCard('reply')">
          <div class="w-10 h-10 rounded-lg bg-blue-100 dark:bg-blue-900 flex items-center justify-center mr-4">
            <div class="i-carbon-continue text-lg text-blue-600 dark:text-blue-400" />
          </div>
          <div class="flex-1">
            <div class="text-base font-medium tracking-tight">
              继续回复设置
            </div>
            <div class="text-xs opacity-60 font-normal mt-0.5">
              配置AI继续回复的行为
            </div>
          </div>
          <div class="i-carbon-chevron-down text-lg opacity-40 transition-transform duration-200" :class="{ 'rotate-180': isExpanded('reply') }" />
        </div>
      </template>
      <n-collapse-transition :show="isExpanded('reply')">
        <ReplySettings />
      </n-collapse-transition>
    </n-card>

    <!-- 窗口设置 -->
    <n-card size="small" hoverable>
      <template #header>
        <div class="flex items-center cursor-pointer" @click="toggleCard('window')">
          <div class="w-10 h-10 rounded-lg bg-green-100 dark:bg-green-900 flex items-center justify-center mr-4">
            <div class="i-carbon-application text-lg text-green-600 dark:text-green-400" />
          </div>
          <div class="flex-1">
            <div class="text-base font-medium tracking-tight">
              窗口设置
            </div>
            <div class="text-xs opacity-60 font-normal mt-0.5">
              调整窗口显示和行为
            </div>
          </div>
          <div class="i-carbon-chevron-down text-lg opacity-40 transition-transform duration-200" :class="{ 'rotate-180': isExpanded('window') }" />
        </div>
      </template>
      <n-collapse-transition :show="isExpanded('window')">
        <WindowSettings
          :always-on-top="alwaysOnTop"
          :window-width="windowWidth"
          :window-height="windowHeight"
          :fixed-window-size="fixedWindowSize"
          @toggle-always-on-top="$emit('toggleAlwaysOnTop')"
          @update-window-size="handleWindowSizeUpdate"
        />
      </n-collapse-transition>
    </n-card>

    <!-- 图片压缩 -->
    <n-card size="small" hoverable>
      <template #header>
        <div class="flex items-center cursor-pointer" @click="toggleCard('image-compression')">
          <div class="w-10 h-10 rounded-lg bg-pink-100 dark:bg-pink-900 flex items-center justify-center mr-4">
            <div class="i-carbon-image text-lg text-pink-600 dark:text-pink-400" />
          </div>
          <div class="flex-1">
            <div class="text-base font-medium tracking-tight">
              图片压缩
            </div>
            <div class="text-xs opacity-60 font-normal mt-0.5">
              粘贴或拖入图片时自动压缩以减小体积
            </div>
          </div>
          <div class="i-carbon-chevron-down text-lg opacity-40 transition-transform duration-200" :class="{ 'rotate-180': isExpanded('image-compression') }" />
        </div>
      </template>
      <n-collapse-transition :show="isExpanded('image-compression')">
        <div class="flex items-center justify-between">
          <div class="flex items-center">
            <div class="w-1.5 h-1.5 bg-info rounded-full mr-3 flex-shrink-0" />
            <div>
              <div class="text-sm font-medium leading-relaxed">
                启用自动压缩
              </div>
              <div class="text-xs opacity-60">
                非透明图片转 JPEG，透明图片保留 PNG，最长边限制 1920px
              </div>
            </div>
          </div>
          <n-switch size="small" :value="imageCompressionEnabled" @update:value="toggleImageCompression" />
        </div>
      </n-collapse-transition>
    </n-card>

    <!-- 音频设置 -->
    <n-card size="small" hoverable>
      <template #header>
        <div class="flex items-center cursor-pointer" @click="toggleCard('audio')">
          <div class="w-10 h-10 rounded-lg bg-yellow-100 dark:bg-yellow-900 flex items-center justify-center mr-4">
            <div class="i-carbon-volume-up text-lg text-yellow-600 dark:text-yellow-400" />
          </div>
          <div class="flex-1">
            <div class="text-base font-medium tracking-tight">
              音频设置
            </div>
            <div class="text-xs opacity-60 font-normal mt-0.5">
              配置音频通知和提示音
            </div>
          </div>
          <div class="i-carbon-chevron-down text-lg opacity-40 transition-transform duration-200" :class="{ 'rotate-180': isExpanded('audio') }" />
        </div>
      </template>
      <n-collapse-transition :show="isExpanded('audio')">
        <AudioSettings
          :audio-notification-enabled="audioNotificationEnabled"
          :audio-url="audioUrl"
          @toggle-audio-notification="$emit('toggleAudioNotification')"
          @update-audio-url="$emit('updateAudioUrl', $event)"
          @test-audio="$emit('testAudio')"
          @stop-audio="$emit('stopAudio')"
          @test-audio-error="$emit('testAudioError', $event)"
        />
      </n-collapse-transition>
    </n-card>

    <!-- Telegram设置 -->
    <n-card size="small" hoverable>
      <template #header>
        <div class="flex items-center cursor-pointer" @click="toggleCard('telegram')">
          <div class="w-10 h-10 rounded-lg bg-cyan-100 dark:bg-cyan-900 flex items-center justify-center mr-4">
            <div class="i-carbon-send text-lg text-cyan-600 dark:text-cyan-400" />
          </div>
          <div class="flex-1">
            <div class="text-base font-medium tracking-tight">
              Telegram设置
            </div>
            <div class="text-xs opacity-60 font-normal mt-0.5">
              配置Telegram机器人集成
            </div>
          </div>
          <div class="i-carbon-chevron-down text-lg opacity-40 transition-transform duration-200" :class="{ 'rotate-180': isExpanded('telegram') }" />
        </div>
      </template>
      <n-collapse-transition :show="isExpanded('telegram')">
        <TelegramSettings />
      </n-collapse-transition>
    </n-card>

    <!-- 快捷模板设置 -->
    <n-card size="small" hoverable>
      <template #header>
        <div class="flex items-center cursor-pointer" @click="toggleCard('custom-prompt')">
          <div class="w-10 h-10 rounded-lg bg-orange-100 dark:bg-orange-900 flex items-center justify-center mr-4">
            <div class="i-carbon-text-creation text-lg text-orange-600 dark:text-orange-400" />
          </div>
          <div class="flex-1">
            <div class="text-base font-medium tracking-tight">
              提示词模板
            </div>
            <div class="text-xs opacity-60 font-normal mt-0.5">
              管理快捷模板和上下文追加
            </div>
          </div>
          <div class="i-carbon-chevron-down text-lg opacity-40 transition-transform duration-200" :class="{ 'rotate-180': isExpanded('custom-prompt') }" />
        </div>
      </template>
      <n-collapse-transition :show="isExpanded('custom-prompt')">
        <CustomPromptSettings />
      </n-collapse-transition>
    </n-card>

    <!-- 快捷键设置 -->
    <n-card size="small" hoverable>
      <template #header>
        <div class="flex items-center cursor-pointer" @click="toggleCard('shortcuts')">
          <div class="w-10 h-10 rounded-lg bg-indigo-100 dark:bg-indigo-900 flex items-center justify-center mr-4">
            <div class="i-carbon-keyboard text-lg text-indigo-600 dark:text-indigo-400" />
          </div>
          <div class="flex-1">
            <div class="text-base font-medium tracking-tight">
              快捷键设置
            </div>
            <div class="text-xs opacity-60 font-normal mt-0.5">
              自定义应用快捷键绑定
            </div>
          </div>
          <div class="i-carbon-chevron-down text-lg opacity-40 transition-transform duration-200" :class="{ 'rotate-180': isExpanded('shortcuts') }" />
        </div>
      </template>
      <n-collapse-transition :show="isExpanded('shortcuts')">
        <ShortcutSettings />
      </n-collapse-transition>
    </n-card>

    <!-- 代理设置 -->
    <n-card size="small" hoverable>
      <template #header>
        <div class="flex items-center cursor-pointer" @click="toggleCard('proxy')">
          <div class="w-10 h-10 rounded-lg bg-cyan-100 dark:bg-cyan-900 flex items-center justify-center mr-4">
            <div class="i-carbon-network-3 text-lg text-cyan-600 dark:text-cyan-400" />
          </div>
          <div class="flex-1">
            <div class="text-base font-medium tracking-tight">
              代理设置
            </div>
            <div class="text-xs opacity-60 font-normal mt-0.5">
              配置更新检查的网络代理
            </div>
          </div>
          <div class="i-carbon-chevron-down text-lg opacity-40 transition-transform duration-200" :class="{ 'rotate-180': isExpanded('proxy') }" />
        </div>
      </template>
      <n-collapse-transition :show="isExpanded('proxy')">
        <ProxySettings />
      </n-collapse-transition>
    </n-card>

    <!-- 项目索引管理 -->
    <n-card size="small" hoverable>
      <template #header>
        <div class="flex items-center cursor-pointer" @click="toggleCard('project-index')">
          <div class="w-10 h-10 rounded-lg bg-teal-100 dark:bg-teal-900 flex items-center justify-center mr-4">
            <div class="i-carbon-data-base text-lg text-teal-600 dark:text-teal-400" />
          </div>
          <div class="flex-1">
            <div class="text-base font-medium tracking-tight">
              项目索引管理
            </div>
            <div class="text-xs opacity-60 font-normal mt-0.5">
              管理和监控项目的索引状态
            </div>
          </div>
          <div class="i-carbon-chevron-down text-lg opacity-40 transition-transform duration-200" :class="{ 'rotate-180': isExpanded('project-index') }" />
        </div>
      </template>
      <n-collapse-transition :show="isExpanded('project-index')">
        <ProjectIndexManager />
      </n-collapse-transition>
    </n-card>

    <!-- 配置管理 -->
    <n-card size="small" hoverable>
      <template #header>
        <div class="flex items-center cursor-pointer" @click="toggleCard('config')">
          <div class="w-10 h-10 rounded-lg bg-blue-100 dark:bg-blue-900 flex items-center justify-center mr-4">
            <div class="i-carbon-settings-adjust text-lg text-blue-600 dark:text-blue-400" />
          </div>
          <div class="flex-1">
            <div class="text-base font-medium tracking-tight">
              配置管理
            </div>
            <div class="text-xs opacity-60 font-normal mt-0.5">
              重新加载配置文件和管理设置
            </div>
          </div>
          <div class="i-carbon-chevron-down text-lg opacity-40 transition-transform duration-200" :class="{ 'rotate-180': isExpanded('config') }" />
        </div>
      </template>
      <n-collapse-transition :show="isExpanded('config')">
        <n-space vertical size="large">
          <div class="flex items-center justify-between">
            <div class="flex items-center">
              <div class="w-1.5 h-1.5 bg-info rounded-full mr-3 flex-shrink-0" />
              <div>
                <div class="text-sm font-medium leading-relaxed">
                  重新加载配置文件
                </div>
                <div class="text-xs opacity-60">
                  从 config.json 重新加载所有配置设置
                </div>
              </div>
            </div>
            <n-button
              size="small"
              type="primary"
              :loading="isReloading"
              @click="reloadConfig"
            >
              <template #icon>
                <div class="i-carbon-restart w-4 h-4" />
              </template>
              重新加载
            </n-button>
          </div>

          <div class="flex items-start">
            <div class="w-1.5 h-1.5 bg-warning rounded-full mr-3 flex-shrink-0 mt-2" />
            <div class="flex-1">
              <div class="text-sm font-medium leading-relaxed mb-2">
                配置文件位置
              </div>
              <n-card size="small" :bordered="true" content-style="padding: 8px 12px">
                <n-text code class="text-xs break-all">{{ configFilePath }}</n-text>
              </n-card>
              <div class="text-xs opacity-60 mt-2">
                您可以直接编辑该文件，然后点击"重新加载"按钮使更改生效
              </div>
            </div>
          </div>
        </n-space>
      </n-collapse-transition>
    </n-card>

  </div>
</template>

