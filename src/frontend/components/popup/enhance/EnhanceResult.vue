<script setup lang="ts">
import { computed } from 'vue'

interface Props {
  isEnhancing: boolean
  hasCompleted: boolean
  errorMessage: string
  streamContent: string
  enhancedPrompt: string
  progress: number
  statusText: string
  projectRootPath: string
  blobCount: number | null
  historyCount: number | null
  // 后端历史加载失败原因（为空表示未失败/未返回）
  historyLoadError?: string
  // 后端是否启用了“历史为空兜底”
  historyFallbackUsed?: boolean
  blobSourceRoot: string
}

const props = defineProps<Props>()

const cardClass = computed(() => {
  if (props.errorMessage) {
    return 'border-error/30 bg-error/5'
  }
  if (props.hasCompleted) {
    return 'border-success/30 bg-success/5'
  }
  return 'border-border bg-container'
})

const statusIconClass = computed(() => {
  if (props.errorMessage) {
    return 'i-carbon-warning-alt text-error'
  }
  if (props.hasCompleted) {
    return 'i-carbon-checkmark-filled text-success'
  }
  return 'i-carbon-magic-wand text-on-surface-muted'
})

const showSkeleton = computed(() => props.isEnhancing && !props.streamContent)

// 判断是否有需要显示的内容（用于控制滚动区域渲染）
const hasContent = computed(() => {
  return props.errorMessage || (props.hasCompleted && props.enhancedPrompt) || props.streamContent || showSkeleton.value
})

const blobCountText = computed(() => {
  if (props.blobCount === null) {
    return '未返回'
  }
  return `已加载 ${props.blobCount} 个代码块`
})

const historyCountText = computed(() => {
  // 中文注释：优先展示“加载失败”，避免将失败误判为“历史为空”
  if (props.historyLoadError) {
    return `加载失败：${props.historyLoadError}`
  }
  if (props.historyCount === null) {
    return '未返回'
  }
  if (props.historyCount === 0) {
    return props.historyFallbackUsed
      ? '历史为空（已使用当前输入作为临时上下文）'
      : '历史为空'
  }
  return `已加载 ${props.historyCount} 条记录`
})

const showSourceRoot = computed(() => {
  return !!props.blobSourceRoot
})

/**
 * 中文注释：清理 Windows 长路径前缀（\\?\ 或 //?/），并统一分隔符
 * - 仅影响显示与比对，不影响真实路径
 */
function normalizePathDisplay(value: string) {
  let v = (value || '').trim()
  // 处理 \\?\ 前缀（Windows 扩展路径语法）
  if (v.startsWith('\\\\?\\'))
    v = v.slice(4)
  // 处理 //?/ 前缀（某些 canonicalize/序列化场景会出现）
  if (v.startsWith('//?/'))
    v = v.slice(4)
  // 统一使用正斜杠
  v = v.replace(/\\/g, '/')
  // 去除末尾斜杠，避免误判
  v = v.replace(/\/+$/, '')
  return v
}

// 中文注释：用于路径比对（Windows 路径大小写不敏感）
function normalizePathCompare(value: string) {
  return normalizePathDisplay(value).toLowerCase()
}

const blobSourceRootDisplay = computed(() => normalizePathDisplay(props.blobSourceRoot))

const sourceMismatch = computed(() => {
  if (!props.blobSourceRoot || !props.projectRootPath) {
    return false
  }
  return normalizePathCompare(props.blobSourceRoot) !== normalizePathCompare(props.projectRootPath)
})
</script>

<template>
  <n-card
    size="small"
    bordered
    class="!rounded-[3px] shadow-sm transition-colors"
    :class="cardClass"
  >
    <div class="mb-3 flex items-center justify-between text-xs" role="status" aria-live="polite">
      <div class="flex items-center gap-2 text-on-surface-secondary">
        <div class="w-4 h-4" :class="statusIconClass" />
        <span>{{ statusText }}</span>
      </div>
      <span v-if="isEnhancing" class="text-on-surface-muted">{{ progress }}%</span>
    </div>

    <!-- 诊断信息：项目路径与上下文统计 -->
    <div class="mb-3 space-y-1 text-xs text-on-surface-secondary">
      <div class="flex items-start gap-2">
        <div class="i-carbon-folder h-3.5 w-3.5 text-on-surface-muted" />
        <span class="text-on-surface-muted">项目：</span>
        <span class="break-all text-on-surface">
          {{ projectRootPath || '未提供项目路径' }}
        </span>
      </div>
      <div class="flex items-center gap-2">
        <div class="i-carbon-package h-3.5 w-3.5 text-on-surface-muted" />
        <span class="text-on-surface-muted">代码上下文：</span>
        <span class="text-on-surface">{{ blobCountText }}</span>
      </div>
      <div class="flex items-center gap-2">
        <div class="i-carbon-chat h-3.5 w-3.5 text-on-surface-muted" />
        <span class="text-on-surface-muted">对话历史：</span>
        <span class="text-on-surface">{{ historyCountText }}</span>
      </div>
      <div v-if="showSourceRoot" class="flex items-start gap-2 text-[11px] text-warning">
        <div class="i-carbon-information h-3.5 w-3.5" />
        <span class="text-on-surface-muted">
          索引来源{{ sourceMismatch ? '（与项目路径不一致）' : '' }}：
        </span>
        <span class="break-all">{{ blobSourceRootDisplay }}</span>
      </div>
    </div>

    <n-progress
      v-if="isEnhancing"
      type="line"
      :percentage="progress"
      :height="6"
      :border-radius="3"
      :show-indicator="false"
      status="info"
    />

    <!-- 内容展示区域：添加滚动控制和渐变遮罩 -->
    <div class="relative mt-3">
      <!-- 使用 n-scrollbar 包裹内容区域，max-h-[300px] 限制高度 -->
      <n-scrollbar v-if="hasContent" class="max-h-[300px]">
        <div class="pr-2 pb-6">
          <n-alert
            v-if="errorMessage"
            type="error"
            :bordered="false"
            class="!rounded-[3px] text-sm"
          >
            {{ errorMessage }}
          </n-alert>

          <!-- 成功状态：增强完成后的结果 -->
          <div
            v-else-if="hasCompleted && enhancedPrompt"
            class="whitespace-pre-wrap text-sm text-success"
          >
            {{ enhancedPrompt }}
          </div>

          <!-- 流式内容：实时显示增强过程 -->
          <div
            v-else-if="streamContent"
            class="whitespace-pre-wrap text-sm text-on-surface"
          >
            {{ streamContent }}
            <span v-if="isEnhancing" class="ml-1 inline-block h-4 w-2 animate-pulse rounded-sm bg-gray-400" />
          </div>

          <!-- 骨架屏：等待流式内容开始 -->
          <div v-else-if="showSkeleton" class="space-y-2">
            <n-skeleton height="14px" width="80%" class="animate-pulse" />
            <n-skeleton height="14px" width="92%" class="animate-pulse" />
            <n-skeleton height="14px" width="88%" class="animate-pulse" />
          </div>
        </div>
      </n-scrollbar>

      <!-- 初始状态：准备中 -->
      <div v-else class="flex items-center gap-2 text-xs text-on-surface-muted">
        <n-spin size="small" />
        正在准备增强...
      </div>

      <!-- 底部渐变遮罩：提示内容可继续滚动 -->
      <div
        v-if="hasContent"
        class="pointer-events-none absolute bottom-0 left-0 right-0 h-8 bg-gradient-to-t from-surface/90 to-transparent"
      />
    </div>
  </n-card>
</template>
