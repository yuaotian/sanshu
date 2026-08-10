<script setup lang="ts">
import type { PlanSessionAdapter } from '../../composables/usePlanSession'
import type { PlanStatus } from '../../types/plan'
import { computed, onUnmounted, ref, toRef, watch } from 'vue'
import { usePlanPanelPreferences } from '../../composables/usePlanPanelPreferences'
import { usePlanSession } from '../../composables/usePlanSession'

const props = defineProps<{
  workspace: string
  sessionAdapter?: PlanSessionAdapter
}>()

const workspace = toRef(props, 'workspace')
const panelRef = ref<HTMLElement | null>(null)
const settingsAreaRef = ref<HTMLElement | null>(null)
const noteInputRef = ref<HTMLInputElement | null>(null)

const {
  clearLocalNote,
  floatingStyle,
  handleDragging,
  isCollapsed,
  isDragging,
  isFloating,
  isEditingNote,
  localNote,
  noteDraft,
  saveLocalNote,
  startNoteEditing,
  cancelNoteEditing,
  settingsOpen,
  showAllItems,
  startDragging,
  stopDragging,
} = usePlanPanelPreferences({ workspace, panelRef, settingsAreaRef })

const {
  allCompleted,
  completed,
  items,
  loading,
  progressPercent,
  readError,
  realtimeError,
  retry,
  snapshot,
  total,
} = usePlanSession({ workspace, adapter: props.sessionAdapter })

const completedTailLimit = 4
// 中文说明：仅在展示层收起较早完成项，始终保留未完成项，不改变真实计划快照。
const hiddenCompletedCount = computed(() => Math.max(0, items.value.filter(item => item.status === 'completed').length - completedTailLimit))
const visibleItems = computed(() => {
  if (showAllItems.value || hiddenCompletedCount.value === 0)
    return items.value

  let hidden = hiddenCompletedCount.value
  return items.value.filter((item) => {
    if (item.status !== 'completed')
      return true
    if (hidden > 0) {
      hidden -= 1
      return false
    }
    return true
  })
})
const remaining = computed(() => Math.max(0, total.value - completed.value))
const canSaveNote = computed(() => noteDraft.value.trim().length > 0)

function editLocalNote(): void {
  startNoteEditing()
  noteInputRef.value?.focus()
}

function closeSettingsWhenPanelContentClicked(event: MouseEvent): void {
  if (!settingsOpen.value || !settingsAreaRef.value)
    return
  if (event.target instanceof Node && settingsAreaRef.value.contains(event.target))
    return
  settingsOpen.value = false
}

interface PanelStatusMeta {
  label: string
  icon: string
  className: string
}

const panelStatus = computed<PanelStatusMeta>(() => {
  if (loading.value) {
    return {
      label: '读取中',
      icon: 'i-carbon-circle-dash animate-spin motion-reduce:animate-none',
      className: 'border-gray-500/30 bg-container-tertiary text-on-surface-secondary',
    }
  }
  if (readError.value) {
    return {
      label: '读取失败',
      icon: 'i-carbon-warning-alt',
      className: 'border-red-500/25 bg-red-500/10 text-red-700 dark:text-red-300',
    }
  }
  if (realtimeError.value) {
    return {
      label: '实时中断',
      icon: 'i-carbon-warning',
      className: 'border-yellow-500/25 bg-yellow-500/10 text-yellow-700 dark:text-yellow-300',
    }
  }
  if (items.value.length === 0) {
    return {
      label: '暂无计划',
      icon: 'i-carbon-list-boxes',
      className: 'border-gray-500/30 bg-container-tertiary text-on-surface-secondary',
    }
  }
  if (allCompleted.value) {
    return {
      label: '已完成',
      icon: 'i-carbon-checkmark-filled',
      className: 'border-green-500/25 bg-green-500/10 text-green-700 dark:text-green-300',
    }
  }
  return {
    label: '进行中',
    icon: 'i-carbon-circle-dash',
    className: 'border-primary-500/25 bg-primary-500/10 text-primary-700 dark:text-primary-300',
  }
})

const liveStatus = computed(() => {
  if (loading.value)
    return '正在读取执行计划'
  if (readError.value)
    return `执行计划读取失败：${readError.value}`
  if (realtimeError.value)
    return '执行计划实时刷新中断，可手动重试'
  if (items.value.length === 0)
    return '当前工作区暂无执行计划'
  if (allCompleted.value)
    return `执行计划已全部完成，共 ${total.value} 项`
  return `执行计划完成度 ${progressPercent.value}%，剩余 ${remaining.value} 项`
})

function statusIcon(status: PlanStatus): string {
  if (status === 'completed')
    return 'i-carbon-checkmark-filled text-green-600 dark:text-green-400'
  if (status === 'in_progress')
    return 'i-carbon-circle-dash text-primary-600 dark:text-primary-400'
  return 'i-carbon-radio-button text-on-surface-secondary'
}

function statusLabel(status: PlanStatus): string {
  if (status === 'completed')
    return '已完成'
  if (status === 'in_progress')
    return '进行中'
  return '待开始'
}

function statusLabelClass(status: PlanStatus): string {
  if (status === 'completed')
    return 'text-green-700 dark:text-green-300'
  if (status === 'in_progress')
    return 'text-primary-700 dark:text-primary-300'
  return 'text-on-surface-secondary'
}

const completedAnimationId = ref('')
let animationTimer: ReturnType<typeof setTimeout> | null = null
let previousStatuses = new Map<string, PlanStatus>()

watch(snapshot, (nextSnapshot) => {
  if (!nextSnapshot) {
    previousStatuses.clear()
    completedAnimationId.value = ''
    return
  }

  const newlyCompleted = nextSnapshot.items.find(item =>
    item.status === 'completed'
    && previousStatuses.has(item.id)
    && previousStatuses.get(item.id) !== 'completed',
  )
  previousStatuses = new Map(nextSnapshot.items.map(item => [item.id, item.status]))

  if (!newlyCompleted)
    return
  completedAnimationId.value = newlyCompleted.id
  if (animationTimer)
    clearTimeout(animationTimer)
  animationTimer = setTimeout(() => {
    completedAnimationId.value = ''
  }, 260)
})

onUnmounted(() => {
  if (animationTimer)
    clearTimeout(animationTimer)
})
</script>

<template>
  <section
    ref="panelRef"
    class="plan-panel rounded-lg border border-gray-300/70 bg-container shadow-sm"
    :class="[
      isFloating ? 'plan-panel--floating' : '',
      isDragging ? 'plan-panel--dragging' : '',
    ]"
    :style="floatingStyle"
    data-guide="plan-panel"
    aria-label="执行计划"
    :aria-busy="loading"
    @pointermove="handleDragging"
    @pointerup="stopDragging"
    @pointercancel="stopDragging"
    @click="closeSettingsWhenPanelContentClicked"
    @keydown.esc.stop="settingsOpen = false"
  >
    <span class="sr-only" aria-live="polite">{{ liveStatus }}</span>

    <header class="relative px-2.5 py-2">
      <div class="flex flex-wrap items-center gap-x-2 gap-y-1.5">
        <button
          type="button"
          class="plan-reset-button min-w-0 flex flex-1 items-center gap-2 rounded px-1.5 py-1 text-left text-xs text-on-surface transition-colors hover:bg-container-secondary focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-primary-500/45"
          :aria-expanded="!isCollapsed"
          @click="isCollapsed = !isCollapsed"
        >
          <div
            class="h-3 w-3 shrink-0 text-primary-500 transition-transform duration-200 motion-reduce:transition-none"
            :class="isCollapsed ? 'i-carbon-chevron-right' : 'i-carbon-chevron-down'"
            aria-hidden="true"
          />
          <div class="i-carbon-list-checked h-3.5 w-3.5 shrink-0 text-primary-500" aria-hidden="true" />
          <span class="truncate font-semibold">执行计划</span>
          <span class="shrink-0 text-[11px] font-medium tabular-nums text-on-surface-secondary">
            {{ completed }}/{{ total }}
          </span>
        </button>

        <div class="ml-auto flex shrink-0 items-center gap-1">
          <span
            class="inline-flex h-6 items-center gap-1 rounded border px-1.5 text-[11px] font-medium whitespace-nowrap"
            :class="panelStatus.className"
          >
            <div class="h-3 w-3 shrink-0" :class="panelStatus.icon" aria-hidden="true" />
            <span>{{ panelStatus.label }}</span>
          </span>

          <button
            v-if="isFloating"
            type="button"
            class="plan-reset-button plan-icon-button plan-drag-handle"
            aria-label="拖动执行计划"
            title="拖动执行计划"
            @pointerdown.stop="startDragging"
          >
            <div class="i-carbon-drag-vertical h-3.5 w-3.5" aria-hidden="true" />
          </button>

          <div ref="settingsAreaRef" class="relative">
            <button
              id="plan-panel-settings-trigger"
              type="button"
              class="plan-reset-button plan-icon-button"
              :aria-expanded="settingsOpen"
              aria-controls="plan-panel-settings"
              aria-label="执行计划设置"
              title="执行计划设置"
              @click.stop="settingsOpen = !settingsOpen"
            >
              <div class="i-carbon-settings h-3.5 w-3.5" aria-hidden="true" />
            </button>

            <div
              v-if="settingsOpen"
              id="plan-panel-settings"
              class="absolute right-0 top-full z-30 mt-1 w-52 rounded-md border border-gray-300 bg-surface p-3 shadow-lg"
              role="group"
              aria-labelledby="plan-panel-settings-label"
            >
              <div class="flex items-center justify-between gap-3">
                <div class="min-w-0">
                  <div id="plan-panel-settings-label" class="text-xs font-medium text-on-surface">
                    窗口内悬浮
                  </div>
                  <div class="mt-0.5 text-[11px] leading-4 text-on-surface-secondary">
                    位置按工作区保存
                  </div>
                </div>
                <n-switch v-model:value="isFloating" size="small" aria-label="窗口内悬浮" />
              </div>
            </div>
          </div>

          <n-tooltip v-if="readError || realtimeError">
            <template #trigger>
              <button
                type="button"
                class="plan-reset-button plan-icon-button text-primary-500"
                aria-label="重新读取执行计划"
                @click="retry"
              >
                <div class="i-carbon-renew h-3.5 w-3.5" aria-hidden="true" />
              </button>
            </template>
            重新读取执行计划
          </n-tooltip>
        </div>
      </div>
    </header>

    <div v-if="!isCollapsed" class="border-t border-gray-300/60 px-2.5 py-2.5">
      <div v-if="loading" class="min-h-10 flex items-center gap-2 px-1 text-xs text-on-surface-secondary">
        <div class="i-carbon-circle-dash h-3.5 w-3.5 animate-spin motion-reduce:animate-none" aria-hidden="true" />
        <span>正在读取计划...</span>
      </div>

      <div v-else-if="readError" class="rounded-md border border-red-500/25 bg-red-500/10 p-2.5 text-xs text-red-700 dark:text-red-300">
        <div class="flex items-start gap-2">
          <div class="i-carbon-warning-alt mt-0.5 h-3.5 w-3.5 shrink-0" aria-hidden="true" />
          <div class="min-w-0 break-words [overflow-wrap:anywhere]">
            <div class="font-medium">
              计划读取失败
            </div>
            <div class="mt-0.5 opacity-80">
              {{ readError }}
            </div>
          </div>
        </div>
      </div>

      <template v-else>
        <div v-if="realtimeError" class="mb-2 flex items-start gap-2 rounded-md border border-yellow-500/25 bg-yellow-500/10 p-2 text-xs text-yellow-700 dark:text-yellow-300">
          <div class="i-carbon-warning mt-0.5 h-3.5 w-3.5 shrink-0" aria-hidden="true" />
          <div class="min-w-0 break-words [overflow-wrap:anywhere]">
            实时刷新暂不可用，当前内容仍可阅读；可使用标题栏重试按钮重新连接。
          </div>
        </div>

        <div v-if="items.length === 0" class="min-h-16 flex flex-col items-center justify-center gap-1.5 px-2 text-center text-xs text-on-surface-secondary">
          <div class="i-carbon-list-boxes h-5 w-5 opacity-70" aria-hidden="true" />
          <span>当前工作区暂无执行计划</span>
        </div>

        <template v-else>
          <div class="px-1 pb-2.5">
            <div class="mb-1.5 flex items-center justify-between gap-2 text-[11px] text-on-surface-secondary">
              <span>{{ allCompleted ? '计划已全部完成' : `剩余 ${remaining} 项` }}</span>
              <span class="shrink-0 font-semibold tabular-nums text-on-surface">
                {{ progressPercent }}%
              </span>
            </div>
            <div class="h-1 overflow-hidden rounded-full bg-container-tertiary" role="progressbar" aria-label="执行计划完成度" :aria-valuenow="progressPercent" aria-valuemin="0" aria-valuemax="100">
              <div
                class="h-full rounded-full bg-primary-500 transition-[width] duration-200 ease-out motion-reduce:transition-none"
                :class="allCompleted ? '!bg-green-500' : ''"
                :style="{ width: `${progressPercent}%` }"
              />
            </div>
          </div>

          <button
            v-if="hiddenCompletedCount > 0"
            type="button"
            class="plan-reset-button mb-1 inline-flex items-center gap-1 rounded px-1.5 py-1 text-[11px] text-on-surface-secondary transition-colors hover:bg-container-secondary hover:text-on-surface focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-primary-500/45"
            :aria-expanded="showAllItems"
            @click="showAllItems = !showAllItems"
          >
            <div class="h-3 w-3" :class="showAllItems ? 'i-carbon-chevron-up' : 'i-carbon-chevron-down'" aria-hidden="true" />
            <span>{{ showAllItems ? '收起较早完成项' : `显示此前 ${hiddenCompletedCount} 个已完成项` }}</span>
          </button>

          <div class="plan-list-scroll max-h-60 overflow-y-auto pr-1 scrollbar-thin">
            <ol class="space-y-0.5">
              <li
                v-for="item in visibleItems"
                :key="item.id"
                class="plan-item min-w-0 flex items-start gap-2 rounded-md border border-transparent px-2 py-1.5 transition-colors duration-150 hover:bg-container-secondary"
                :class="[
                  item.status === 'completed' ? 'opacity-75' : '',
                  item.status === 'in_progress' ? 'border-primary-500/25 bg-primary-500/10' : '',
                  completedAnimationId === item.id ? 'plan-item-completed-now' : '',
                ]"
                :aria-current="item.status === 'in_progress' ? 'step' : undefined"
              >
                <div
                  class="plan-status-icon mt-0.5 h-3.5 w-3.5 shrink-0"
                  :class="statusIcon(item.status)"
                  aria-hidden="true"
                />
                <div class="min-w-0 flex flex-1 flex-wrap items-baseline gap-x-2 gap-y-0.5">
                  <span
                    class="min-w-0 break-words text-xs leading-5 text-on-surface [overflow-wrap:anywhere]"
                    :class="item.status === 'completed' ? 'line-through decoration-gray-500/60' : ''"
                  >
                    {{ item.text }}
                  </span>
                  <span class="shrink-0 text-[11px] leading-4" :class="statusLabelClass(item.status)">
                    {{ statusLabel(item.status) }}
                  </span>
                </div>
              </li>
            </ol>
          </div>

          <div class="mt-2.5 border-t border-gray-300/50 pt-2.5">
            <div v-if="localNote" class="mb-1.5 flex items-start gap-2 rounded-md bg-container-secondary px-2 py-1.5 text-xs text-on-surface-secondary">
              <div class="i-carbon-notebook mt-0.5 h-3.5 w-3.5 shrink-0 text-primary-500" aria-hidden="true" />
              <span class="min-w-0 flex-1 break-words [overflow-wrap:anywhere]">{{ localNote }}</span>
              <button
                type="button"
                class="plan-reset-button inline-flex h-5 w-5 shrink-0 items-center justify-center rounded text-on-surface-secondary transition-colors hover:bg-container-tertiary hover:text-on-surface focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-primary-500/45"
                aria-label="编辑本地备注"
                title="编辑本地备注"
                @click="editLocalNote"
              >
                <div class="i-carbon-edit h-3 w-3" aria-hidden="true" />
              </button>
              <button
                type="button"
                class="plan-reset-button inline-flex h-5 w-5 shrink-0 items-center justify-center rounded text-on-surface-secondary transition-colors hover:bg-container-tertiary hover:text-on-surface focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-primary-500/45"
                aria-label="清除本地备注"
                title="清除本地备注"
                @click="clearLocalNote"
              >
                <div class="i-carbon-close h-3 w-3" aria-hidden="true" />
              </button>
            </div>

            <div class="flex items-center gap-1.5 rounded-md border border-gray-300/70 bg-surface px-2 py-1.5 focus-within:border-primary-500/60 focus-within:ring-1 focus-within:ring-primary-500/25">
              <div class="i-carbon-edit h-3.5 w-3.5 shrink-0 text-primary-500" aria-hidden="true" />
              <input
                ref="noteInputRef"
                v-model="noteDraft"
                type="text"
                class="min-w-0 flex-1 border-0 bg-transparent p-0 text-xs leading-5 text-on-surface outline-none placeholder:text-on-surface-muted"
                aria-label="添加执行计划本地备注"
                :placeholder="isEditingNote ? '修改本地备注' : '添加本地备注'"
                @keydown.enter.prevent="saveLocalNote"
                @keydown.esc.stop.prevent="cancelNoteEditing"
              >
              <button
                v-if="isEditingNote"
                type="button"
                class="plan-reset-button inline-flex h-5 w-5 shrink-0 items-center justify-center rounded text-on-surface-secondary transition-colors hover:bg-container-secondary hover:text-on-surface focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-primary-500/45"
                aria-label="取消编辑本地备注"
                title="取消编辑本地备注"
                @click="cancelNoteEditing"
              >
                <div class="i-carbon-close h-3 w-3" aria-hidden="true" />
              </button>
              <button
                type="button"
                class="plan-reset-button inline-flex h-5 w-5 shrink-0 items-center justify-center rounded text-primary-500 transition-colors hover:bg-container-secondary focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-primary-500/45 disabled:cursor-not-allowed disabled:opacity-35"
                :disabled="!canSaveNote"
                aria-label="保存本地备注"
                title="保存本地备注"
                @click="saveLocalNote"
              >
                <div class="i-carbon-save h-3 w-3" aria-hidden="true" />
              </button>
            </div>
          </div>
        </template>
      </template>
    </div>
  </section>
</template>

<style scoped>
.plan-reset-button {
  appearance: none;
  border: 0;
  background: transparent;
  color: inherit;
  font: inherit;
}

.plan-icon-button {
  display: inline-flex;
  width: 1.75rem;
  height: 1.75rem;
  flex: 0 0 1.75rem;
  align-items: center;
  justify-content: center;
  border-radius: 0.25rem;
  color: var(--color-on-surface-secondary);
  transition: color 150ms ease, background-color 150ms ease;
}

.plan-icon-button:hover {
  color: var(--color-on-surface);
  background: var(--color-container-secondary);
}

.plan-icon-button:focus-visible {
  outline: 2px solid rgba(20, 184, 166, 0.55);
  outline-offset: 1px;
}

.plan-panel--floating {
  position: fixed;
  right: 1rem;
  bottom: 1rem;
  z-index: 40;
  width: min(360px, calc(100vw - 1.5rem));
  max-width: calc(100vw - 1.5rem);
  max-height: calc(100vh - 1.5rem);
  overflow: auto;
  box-shadow: 0 14px 36px rgba(0, 0, 0, 0.24);
}

.plan-panel--dragging {
  user-select: none;
}

.plan-drag-handle {
  touch-action: none;
  cursor: grab;
}

.plan-panel--dragging .plan-drag-handle {
  cursor: grabbing;
}

@keyframes plan-completed {
  0% { transform: scale(0.8); opacity: 0.4; }
  60% { transform: scale(1.15); opacity: 1; }
  100% { transform: scale(1); opacity: 1; }
}

.plan-item-completed-now .plan-status-icon {
  animation: plan-completed 220ms ease-out;
}

@media (max-width: 420px) {
  .plan-panel {
    max-width: 100%;
  }
}

@media (prefers-reduced-motion: reduce) {
  .plan-icon-button {
    transition: none;
  }

  .plan-item-completed-now .plan-status-icon {
    animation: none;
  }
}
</style>
