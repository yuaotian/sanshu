<script setup lang="ts">
import ThemeIcon from '../common/ThemeIcon.vue'

interface Props {
  currentTheme?: string
  loading?: boolean
  showMainLayout?: boolean
  alwaysOnTop?: boolean
  /** 是否启用了 sou 代码搜索工具，用于控制 MCP 索引状态指示器的显示 */
  mcpEnabled?: boolean
  /** 当前项目的索引状态摘要文本（例如：已同步 / 索引中 xx%） */
  mcpStatusSummary?: string
  /** 当前项目的索引状态图标类名（由 useAcemcpSync 提供） */
  mcpStatusIcon?: string
  /** 是否正在进行索引，用于控制指示器的 loading 态 */
  mcpIsIndexing?: boolean
  /** 最近一次索引失败的时间戳（由后端提供的原始字符串） */
  mcpLastFailureTime?: string | null
  /** 最近一次索引失败的错误信息摘要 */
  mcpLastError?: string | null
  /** 当前项目失败文件数量，用于快速告警提示 */
  mcpFailedFiles?: number
}

interface Emits {
  themeChange: [theme: string]
  openMainLayout: []
  /** 打开实时日志查看器 */
  openLogViewer: []
  toggleAlwaysOnTop: []
  /** 打开 MCP 代码索引详情抽屉 */
  openIndexStatus: []
}

const props = withDefaults(defineProps<Props>(), {
  currentTheme: 'dark',
  loading: false,
  showMainLayout: false,
  alwaysOnTop: false,
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
  // 切换到下一个主题
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
  // 仅在 sou 工具启用且存在有效状态时响应点击
  if (!props.mcpEnabled)
    return
  emit('openIndexStatus')
}
</script>

<template>
  <div class="px-4 py-3 select-none">
    <div class="flex items-center justify-between gap-3">
      <!-- 左侧：标题 -->
      <div class="flex items-center gap-3 min-w-0">
        <div class="w-3 h-3 rounded-full bg-primary-500" />
        <h1 class="text-base font-medium text-slate-800 dark:text-white truncate">
          三术 - 道生一，一生二，二生三，三生万物
        </h1>
      </div>

      <!-- 右侧：MCP 索引状态指示器 + 操作按钮 -->
      <div class="flex items-center gap-3">
        <!-- MCP 代码索引状态指示器（仅在 sou 工具启用且有项目索引状态时显示） -->
        <n-tooltip
          v-if="mcpEnabled && mcpStatusSummary"
          trigger="hover"
          placement="bottom"
        >
          <template #trigger>
            <button
              type="button"
              class="inline-flex items-center gap-1.5 rounded-full border border-black-300/60 bg-black-200/70 px-2.5 py-1 text-xs text-white transition-colors duration-150 hover:bg-black-300/70"
              @click="handleOpenIndexStatus"
            >
              <div
                :class="[mcpStatusIcon, mcpIsIndexing ? 'animate-spin-slow' : '']"
                class="w-3.5 h-3.5"
              />
              <span class="font-medium whitespace-nowrap">
                代码索引
              </span>
              <span class="text-[11px] opacity-80 max-w-[120px] truncate">
                {{ mcpStatusSummary }}
              </span>
            </button>
          </template>
          <div class="text-xs space-y-1">
            <div class="font-medium">
              代码索引同步状态
            </div>
            <div>
              当前项目的代码索引由 Acemcp 后台维护，状态会自动轮询更新。
            </div>
            <div v-if="mcpIsIndexing">
              正在索引中，稍后搜索结果会更加完整。
            </div>
            <div v-if="(props.mcpFailedFiles ?? 0) > 0" class="text-red-600 dark:text-red-400">
              最近失败文件数：{{ props.mcpFailedFiles }}
            </div>
            <div v-if="props.mcpLastFailureTime" class="text-red-600 dark:text-red-300">
              最近失败时间：{{ props.mcpLastFailureTime }}
            </div>
            <div v-if="props.mcpLastError" class="text-red-600 dark:text-red-300 line-clamp-3">
              最近错误：{{ props.mcpLastError }}
            </div>
            <div
              v-else-if="!mcpIsIndexing && (props.mcpFailedFiles ?? 0) === 0"
              class="text-green-600 dark:text-green-300"
            >
              最近无错误，索引状态稳定。
            </div>
          </div>
        </n-tooltip>

        <n-space size="small">
          <!-- 置顶按钮 -->
          <n-button
            size="small"
            quaternary
            circle
            :title="props.alwaysOnTop ? '取消置顶' : '窗口置顶'"
            @click="handleToggleAlwaysOnTop"
          >
            <template #icon>
              <div
                :class="props.alwaysOnTop ? 'i-carbon-pin-filled' : 'i-carbon-pin'"
                class="w-4 h-4 text-slate-700 dark:text-white"
              />
            </template>
          </n-button>
          <n-button
            size="small"
            quaternary
            circle
            :title="props.showMainLayout ? '返回聊天' : '打开设置'"
            @click="handleOpenMainLayout"
          >
            <template #icon>
              <div
                :class="props.showMainLayout ? 'i-carbon-chat' : 'i-carbon-settings'"
                class="w-4 h-4 text-slate-700 dark:text-white"
              />
            </template>
          </n-button>
          <n-button
            size="small"
            quaternary
            circle
            title="查看日志"
            @click="handleOpenLogViewer"
          >
            <template #icon>
              <div class="i-carbon-document w-4 h-4 text-slate-700 dark:text-white" />
            </template>
          </n-button>
          <n-button
            size="small"
            quaternary
            circle
            :title="`切换到${props.currentTheme === 'light' ? '深色' : '浅色'}主题`"
            @click="handleThemeChange"
          >
            <template #icon>
              <ThemeIcon :theme="props.currentTheme" class="w-4 h-4 text-slate-700 dark:text-white" />
            </template>
          </n-button>
        </n-space>
      </div>
    </div>
  </div>
</template>
