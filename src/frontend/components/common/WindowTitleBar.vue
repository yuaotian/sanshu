<script setup lang="ts">
import { getCurrentWebviewWindow } from '@tauri-apps/api/webviewWindow'
import ThemeIcon from './ThemeIcon.vue'

interface Props {
  title?: string
  showMinimize?: boolean
  showClose?: boolean
  currentTheme?: string
}

interface Emits {
  themeChange: [theme: string]
}

const props = withDefaults(defineProps<Props>(), {
  title: '三术',
  showMinimize: true,
  showClose: true,
  currentTheme: 'dark',
})

const emit = defineEmits<Emits>()

const appWindow = getCurrentWebviewWindow()

function handleThemeToggle() {
  const next = props.currentTheme === 'light' ? 'dark' : 'light'
  emit('themeChange', next)
}

async function handleMinimize() {
  await appWindow.minimize()
}

async function handleClose() {
  await appWindow.close()
}
</script>

<template>
  <div class="select-none" data-tauri-drag-region>
    <div class="flex items-center justify-between gap-2 px-3 py-1.5" data-tauri-drag-region>
      <!-- 左侧：标题 -->
      <div class="flex items-center gap-2 min-w-0" data-tauri-drag-region>
        <div class="w-2.5 h-2.5 rounded-full bg-primary-500 flex-shrink-0" />
        <span class="text-xs font-medium text-on-surface-secondary truncate" data-tauri-drag-region>
          {{ title }}
        </span>
      </div>

      <!-- 中间：自定义内容插槽 -->
      <div class="flex items-center gap-2 flex-1 justify-end" data-tauri-drag-region>
        <slot />

        <!-- 分隔线（仅在有插槽内容时可见） -->
        <div v-if="$slots.default" class="w-px h-3 shrink-0" style="background-color: color-mix(in srgb, var(--color-on-surface) 25%, transparent)" />

        <!-- 主题切换 + 窗口控制 -->
        <n-space :size="2">
          <n-button
            size="tiny"
            quaternary
            circle
            :title="`切换到${currentTheme === 'light' ? '深色' : '浅色'}主题`"
            @click="handleThemeToggle"
          >
            <template #icon>
              <ThemeIcon :theme="currentTheme" class="w-3.5 h-3.5 text-on-surface-secondary" />
            </template>
          </n-button>
          <n-button
            v-if="showMinimize"
            size="tiny"
            quaternary
            circle
            title="最小化"
            @click="handleMinimize"
          >
            <template #icon>
              <div class="i-carbon-subtract w-3.5 h-3.5 text-on-surface-secondary" />
            </template>
          </n-button>
          <n-button
            v-if="showClose"
            size="tiny"
            quaternary
            circle
            title="关闭"
            class="titlebar-close-btn"
            @click="handleClose"
          >
            <template #icon>
              <div class="i-carbon-close w-3.5 h-3.5 text-on-surface-secondary" />
            </template>
          </n-button>
        </n-space>
      </div>
    </div>
  </div>
</template>

<style scoped>
.titlebar-close-btn:hover {
  background-color: color-mix(in srgb, var(--color-error) 80%, transparent) !important;
  color: #fff !important;
}
</style>
