<script setup lang="ts">
import WindowTitleBar from '../common/WindowTitleBar.vue'

interface Props {
  currentTheme?: string
  loading?: boolean
  showMainLayout?: boolean
  alwaysOnTop?: boolean
  projectName?: string
  mcpEnabled?: boolean
  mcpStatusSummary?: string
  mcpStatusIcon?: string
  mcpIsIndexing?: boolean
  mcpLastFailureTime?: string | null
  mcpLastError?: string | null
  mcpFailedFiles?: number
}

interface Emits {
  themeChange: [theme: string]
  openMainLayout: []
  openLogViewer: []
  toggleAlwaysOnTop: []
  openIndexStatus: []
}

const props = withDefaults(defineProps<Props>(), {
  currentTheme: 'dark',
  loading: false,
  showMainLayout: false,
  alwaysOnTop: false,
  projectName: '',
  mcpEnabled: false,
  mcpStatusSummary: '',
  mcpStatusIcon: 'i-carbon-help text-gray-400',
  mcpIsIndexing: false,
  mcpLastFailureTime: null,
  mcpLastError: null,
  mcpFailedFiles: 0,
})

const emit = defineEmits<Emits>()

function handleThemeChange() {
  const nextTheme = props.currentTheme === 'light' ? 'dark' : 'light'
  emit('themeChange', nextTheme)
}

function handleOpenMainLayout() {
  emit('openMainLayout')
}

function handleOpenLogViewer() {
  emit('openLogViewer')
}

function handleToggleAlwaysOnTop() {
  emit('toggleAlwaysOnTop')
}

function handleOpenIndexStatus() {
  if (!props.mcpEnabled)
    return
  emit('openIndexStatus')
}

function isAuthFailure(): boolean {
  const lastError = props.mcpLastError || ''
  const lower = lastError.toLowerCase()
  return lower.includes('401') || lower.includes('认证失败') || lower.includes('invalid token')
}
</script>

<template>
  <WindowTitleBar :title="props.projectName ? `三术 · ${props.projectName}` : '三术'" :current-theme="props.currentTheme" @theme-change="handleThemeChange">
    <!-- MCP 代码索引状态 -->
    <n-tooltip
      v-if="mcpEnabled && mcpStatusSummary"
      trigger="hover"
      placement="bottom"
    >
      <template #trigger>
        <n-button
          size="tiny"
          quaternary
          class="!px-2 !py-0.5 !rounded-md !border !border-black-300/60 !bg-black-200/70 !text-[11px]"
          @click="handleOpenIndexStatus"
        >
          <template #icon>
            <div
              :class="[mcpStatusIcon, mcpIsIndexing ? 'animate-spin-slow' : '']"
              class="w-3 h-3"
            />
          </template>
          <span class="font-medium whitespace-nowrap">索引</span>
          <span class="opacity-80 max-w-[80px] truncate">{{ mcpStatusSummary }}</span>
        </n-button>
      </template>
      <div class="text-xs space-y-1">
        <div class="font-medium">代码索引同步状态</div>
        <div>当前项目的代码索引由 Acemcp 后台维护，状态会自动轮询更新。</div>
        <div v-if="isAuthFailure()" class="text-error font-medium">
          检测到 ACE Token 认证失败，请点击状态面板后前往设置更新 Token。
        </div>
        <div v-if="mcpIsIndexing">正在索引中，稍后搜索结果会更加完整。</div>
        <div v-if="(props.mcpFailedFiles ?? 0) > 0" class="text-error">
          最近失败文件数：{{ props.mcpFailedFiles }}
        </div>
        <div v-if="props.mcpLastFailureTime" class="text-error">
          最近失败时间：{{ props.mcpLastFailureTime }}
        </div>
        <div v-if="props.mcpLastError" class="text-error line-clamp-3">
          最近错误：{{ props.mcpLastError }}
        </div>
        <div
          v-else-if="!mcpIsIndexing && (props.mcpFailedFiles ?? 0) === 0"
          class="text-success"
        >
          最近无错误，索引状态稳定。
        </div>
      </div>
    </n-tooltip>

    <!-- 功能按钮组 -->
    <n-space :size="2">
      <n-button
        size="tiny"
        quaternary
        circle
        :title="props.alwaysOnTop ? '取消置顶' : '窗口置顶'"
        @click="handleToggleAlwaysOnTop"
      >
        <template #icon>
          <div
            :class="props.alwaysOnTop ? 'i-carbon-pin-filled' : 'i-carbon-pin'"
            class="w-3.5 h-3.5 text-white/70"
          />
        </template>
      </n-button>
      <n-button
        size="tiny"
        quaternary
        circle
        :title="props.showMainLayout ? '返回聊天' : '打开设置'"
        @click="handleOpenMainLayout"
      >
        <template #icon>
          <div
            :class="props.showMainLayout ? 'i-carbon-chat' : 'i-carbon-settings'"
            class="w-3.5 h-3.5 text-white/70"
          />
        </template>
      </n-button>
      <n-button
        size="tiny"
        quaternary
        circle
        title="查看日志"
        @click="handleOpenLogViewer"
      >
        <template #icon>
          <div class="i-carbon-document w-3.5 h-3.5 text-white/70" />
        </template>
      </n-button>
    </n-space>
  </WindowTitleBar>
</template>
