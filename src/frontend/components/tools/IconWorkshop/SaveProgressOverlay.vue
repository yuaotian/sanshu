<script setup lang="ts">
/**
 * 保存进度覆盖层
 * 展示批量保存进度与结果摘要，完成后由用户确认关闭
 * 从 IconPopupMode.vue（原上帝组件）拆分而来
 */
import type { IconSaveResult } from '../../../types/icon'

interface Props {
  /** 是否正在保存 */
  isSaving: boolean
  /** 保存进度百分比 */
  progress: number
  /** 当前正在保存的图标名称 */
  savingIconName: string
  /** 保存结果摘要 */
  summary: IconSaveResult | null
  /** 保存过程中的错误信息 */
  error: string | null
}

defineProps<Props>()

const emit = defineEmits<{
  /** 用户确认结果并关闭弹窗 */
  confirm: []
}>()
</script>

<template>
  <div class="absolute inset-0 z-30 flex items-center justify-center bg-surface backdrop-blur">
    <div class="w-full max-w-xl rounded-2xl border border-border bg-surface-variant p-6 shadow-lg space-y-4">
      <div class="flex items-center gap-3">
        <div class="i-carbon-download text-xl text-primary" />
        <div class="text-base font-medium">
          {{ isSaving ? '正在保存图标...' : '保存完成' }}
        </div>
      </div>

      <div v-if="isSaving" class="space-y-3">
        <div class="flex items-center justify-between text-sm text-on-surface-secondary">
          <span>当前进度</span>
          <span>{{ progress }}%</span>
        </div>
        <n-progress
          type="line"
          :percentage="progress"
          :show-indicator="false"
          processing
        />
        <div class="text-sm text-on-surface-secondary">
          正在处理：{{ savingIconName || '准备中' }}
        </div>
      </div>

      <div v-else class="space-y-3">
        <div class="flex items-center gap-2 text-sm text-on-surface-secondary">
          <div class="i-carbon-checkmark-outline text-green-500" />
          <span>保存任务已完成</span>
        </div>

        <div v-if="summary" class="grid grid-cols-2 gap-3 text-sm">
          <div class="rounded-lg border border-border bg-surface p-3">
            <div class="text-on-surface-secondary">
              成功
            </div>
            <div class="text-lg font-semibold text-green-600">
              {{ summary.successCount }}
            </div>
          </div>
          <div class="rounded-lg border border-border bg-surface p-3">
            <div class="text-on-surface-secondary">
              失败
            </div>
            <div class="text-lg font-semibold text-red-500">
              {{ summary.failedCount }}
            </div>
          </div>
          <div class="col-span-2 rounded-lg border border-border bg-surface p-3">
            <div class="text-on-surface-secondary">
              保存路径
            </div>
            <div class="text-xs mt-1 break-all">
              {{ summary.savePath }}
            </div>
          </div>
        </div>

        <div v-if="error" class="text-xs text-red-500">
          {{ error }}
        </div>

        <div class="flex justify-end">
          <n-button type="primary" @click="emit('confirm')">
            确认并关闭
          </n-button>
        </div>
      </div>
    </div>
  </div>
</template>
