<script setup lang="ts">
import type { ProjectIndexStatus } from '../../types/tauri'
import { computed } from 'vue'

const props = defineProps<{
  show: boolean
  status: ProjectIndexStatus | null
  busy: boolean
}>()

const emit = defineEmits<{
  close: []
  remove: []
  confirm: []
}>()

const risk = computed(() => props.status?.scope_risk ?? null)

function formatCount(value: number): string {
  return new Intl.NumberFormat('zh-CN').format(value)
}

function formatBytes(value: number): string {
  if (value <= 0)
    return '尚未统计'
  const units = ['B', 'KiB', 'MiB', 'GiB']
  let amount = value
  let unitIndex = 0
  while (amount >= 1024 && unitIndex < units.length - 1) {
    amount /= 1024
    unitIndex += 1
  }
  return `${amount.toFixed(unitIndex === 0 ? 0 : 1)} ${units[unitIndex]}`
}
</script>

<template>
  <n-modal
    :show="show && !!risk && !!status"
    :mask-closable="false"
    :close-on-esc="false"
    transform-origin="center"
  >
    <section
      v-if="risk && status"
      class="scope-risk-dialog w-[min(560px,calc(100vw-24px))] overflow-hidden rounded-lg border border-surface-200 bg-container shadow-xl dark:border-surface-700"
      role="alertdialog"
      aria-labelledby="scope-risk-title"
      aria-describedby="scope-risk-description"
    >
      <header class="flex items-start gap-3 border-b border-surface-200 p-5 dark:border-surface-700">
        <div class="flex h-9 w-9 shrink-0 items-center justify-center rounded-lg bg-amber-500/12 text-amber-500">
          <div class="i-carbon-warning-alt h-5 w-5" aria-hidden="true" />
        </div>
        <div class="min-w-0 flex-1">
          <div class="flex flex-wrap items-center gap-2">
            <h2 id="scope-risk-title" class="m-0 text-base font-semibold text-on-surface">
              请确认项目索引范围
            </h2>
            <n-tag size="small" type="warning">
              索引已暂停
            </n-tag>
          </div>
          <p class="mb-0 mt-1 text-xs leading-5 text-on-surface-secondary">
            当前路径可能覆盖大量非项目文件。确认前不会读取文件正文或恢复上传。
          </p>
        </div>
      </header>

      <div class="space-y-4 p-5">
        <n-alert id="scope-risk-description" type="warning" :bordered="false">
          {{ risk.reason }}
        </n-alert>

        <div class="space-y-1.5">
          <div class="text-xs font-medium text-on-surface-secondary">
            项目路径
          </div>
          <div class="break-all rounded-md border border-surface-200 bg-surface px-3 py-2 font-mono text-xs leading-5 text-on-surface dark:border-surface-700">
            {{ status.project_root }}
          </div>
        </div>

        <dl class="grid grid-cols-1 gap-2 sm:grid-cols-3">
          <div class="rounded-md border border-surface-200 px-3 py-2.5 dark:border-surface-700">
            <dt class="text-xs text-on-surface-secondary">
              已扫描目录项
            </dt>
            <dd class="mb-0 ml-0 mt-1 text-sm font-semibold text-on-surface">
              {{ risk.scanned_entries ? formatCount(risk.scanned_entries) : '关键路径命中' }}
            </dd>
          </div>
          <div class="rounded-md border border-surface-200 px-3 py-2.5 dark:border-surface-700">
            <dt class="text-xs text-on-surface-secondary">
              候选文件
            </dt>
            <dd class="mb-0 ml-0 mt-1 text-sm font-semibold text-on-surface">
              {{ risk.candidate_files ? formatCount(risk.candidate_files) : '尚未统计' }}
            </dd>
          </div>
          <div class="rounded-md border border-surface-200 px-3 py-2.5 dark:border-surface-700">
            <dt class="text-xs text-on-surface-secondary">
              候选体积
            </dt>
            <dd class="mb-0 ml-0 mt-1 text-sm font-semibold text-on-surface">
              {{ formatBytes(risk.candidate_bytes) }}
            </dd>
          </div>
        </dl>

        <div v-if="risk.project_markers.length" class="flex flex-wrap items-center gap-2">
          <span class="text-xs text-on-surface-secondary">检测到项目标识</span>
          <n-tag v-for="marker in risk.project_markers" :key="marker" size="small" :bordered="false">
            {{ marker }}
          </n-tag>
        </div>
      </div>

      <footer class="flex flex-col-reverse gap-2 border-t border-surface-200 bg-surface-50 p-4 dark:border-surface-700 sm:flex-row sm:justify-end">
        <n-button :disabled="busy" @click="emit('close')">
          暂不处理
        </n-button>
        <n-button type="error" secondary :disabled="busy" @click="emit('remove')">
          <template #icon>
            <div class="i-carbon-trash-can" />
          </template>
          移除索引记录
        </n-button>
        <n-button type="warning" :loading="busy" @click="emit('confirm')">
          <template #icon>
            <div class="i-carbon-checkmark" />
          </template>
          确认这是项目
        </n-button>
      </footer>
    </section>
  </n-modal>
</template>

<style scoped>
.scope-risk-dialog {
  max-height: calc(100vh - 24px);
  overflow-y: auto;
}
</style>
