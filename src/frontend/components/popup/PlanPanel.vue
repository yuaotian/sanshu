<script setup lang="ts">
import type { UnlistenFn } from '@tauri-apps/api/event'
import type { PlanSnapshot, PlanStatus } from '../../types/plan'
import { invoke } from '@tauri-apps/api/core'
import { listen } from '@tauri-apps/api/event'
import { useStorage } from '@vueuse/core'
import { computed, onMounted, onUnmounted, ref, watch } from 'vue'

const props = defineProps<{
  workspace: string
}>()

const isCollapsed = useStorage('popup-plan-panel-collapsed', true)
const snapshot = ref<PlanSnapshot | null>(null)
const loading = ref(true)
const readError = ref('')
const watchError = ref('')
const eventError = ref('')
const completedAnimationId = ref('')

let mounted = false
let loadSequence = 0
let lifecycleGeneration = 0
let watchGeneration = 0
let unlistenPlanUpdate: UnlistenFn | null = null
let animationTimer: ReturnType<typeof setTimeout> | null = null
let previousStatuses = new Map<string, PlanStatus>()
let listenerSetupPromise: Promise<boolean> | null = null
let watchSetupQueue: Promise<void> = Promise.resolve()

const items = computed(() => snapshot.value?.items ?? [])
const completed = computed(() => snapshot.value?.summary.completed ?? 0)
const total = computed(() => snapshot.value?.summary.total ?? 0)
const allCompleted = computed(() => snapshot.value?.summary.all_completed ?? false)
const progressPercent = computed(() => total.value === 0 ? 0 : Math.round((completed.value / total.value) * 100))
const realtimeError = computed(() => eventError.value || watchError.value)

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

function applySnapshot(nextSnapshot: PlanSnapshot) {
  const newlyCompleted = nextSnapshot.items.find(item =>
    item.status === 'completed'
    && previousStatuses.has(item.id)
    && previousStatuses.get(item.id) !== 'completed',
  )

  snapshot.value = nextSnapshot
  previousStatuses = new Map(nextSnapshot.items.map(item => [item.id, item.status]))

  if (newlyCompleted) {
    completedAnimationId.value = newlyCompleted.id
    if (animationTimer)
      clearTimeout(animationTimer)
    animationTimer = setTimeout(() => {
      completedAnimationId.value = ''
    }, 260)
  }
}

async function loadPlan(showLoading = false, workspace = props.workspace) {
  const sequence = ++loadSequence
  if (showLoading)
    loading.value = true
  readError.value = ''

  try {
    const nextSnapshot = await invoke<PlanSnapshot>('get_plan_snapshot', {
      workspace,
    })
    if (sequence === loadSequence)
      applySnapshot(nextSnapshot)
  }
  catch (error) {
    if (sequence === loadSequence)
      readError.value = String(error)
  }
  finally {
    if (sequence === loadSequence)
      loading.value = false
  }
}

function isCurrentWatch(generation: number): boolean {
  return mounted && generation === watchGeneration
}

function isCurrentLifecycle(generation: number): boolean {
  return mounted && generation === lifecycleGeneration
}

async function stopWorkspaceWatch() {
  try {
    await invoke('stop_plan_watch')
  }
  catch (error) {
    console.warn('停止计划文件监听失败：', error)
  }
}

async function startWorkspaceWatch(generation: number, workspace: string) {
  if (!isCurrentWatch(generation))
    return

  watchError.value = ''
  previousStatuses.clear()
  snapshot.value = null
  loading.value = true

  let started = false
  try {
    await invoke('start_plan_watch', { workspace })
    started = true
  }
  catch (error) {
    if (isCurrentWatch(generation))
      watchError.value = String(error)
  }

  if (!isCurrentWatch(generation)) {
    if (started)
      await stopWorkspaceWatch()
    return
  }

  // 中文说明：监听建立后再读取，覆盖监听启动前发生更新的竞态窗口。
  loadPlan(true, workspace)
}

function queueWorkspaceWatch(generation: number, workspace: string): Promise<void> {
  // 中文说明：串行建立 watcher，过期任务完成后先清理，再启动最新工作区。
  watchSetupQueue = watchSetupQueue.then(() => startWorkspaceWatch(generation, workspace))
  return watchSetupQueue
}

async function retry() {
  await restartWorkspaceWatch()
}

async function ensurePlanListener(): Promise<boolean> {
  if (unlistenPlanUpdate)
    return mounted
  if (listenerSetupPromise)
    return listenerSetupPromise

  const generation = lifecycleGeneration
  const setupPromise = (async () => {
    try {
      const unlisten = await listen('plan-updated', () => {
        if (mounted)
          loadPlan()
      })
      if (!isCurrentLifecycle(generation)) {
        unlisten()
        return false
      }
      unlistenPlanUpdate = unlisten
      eventError.value = ''
      return true
    }
    catch (error) {
      if (!isCurrentLifecycle(generation))
        return false
      eventError.value = String(error)
      return true
    }
  })()
  listenerSetupPromise = setupPromise
  try {
    return await setupPromise
  }
  finally {
    if (listenerSetupPromise === setupPromise)
      listenerSetupPromise = null
  }
}

async function restartWorkspaceWatch() {
  const generation = ++watchGeneration
  const workspace = props.workspace
  loadSequence += 1

  if (await ensurePlanListener()) {
    if (isCurrentWatch(generation))
      await queueWorkspaceWatch(generation, workspace)
  }
}

onMounted(async () => {
  mounted = true
  await restartWorkspaceWatch()
})

watch(() => props.workspace, async (workspace, previousWorkspace) => {
  if (mounted && workspace !== previousWorkspace)
    await restartWorkspaceWatch()
})

onUnmounted(() => {
  mounted = false
  lifecycleGeneration += 1
  watchGeneration += 1
  loadSequence += 1
  if (animationTimer)
    clearTimeout(animationTimer)
  unlistenPlanUpdate?.()
  unlistenPlanUpdate = null
  stopWorkspaceWatch()
})
</script>

<template>
  <section class="space-y-2" data-guide="plan-panel" aria-label="执行计划">
    <div class="flex items-center justify-between gap-2">
      <button
        type="button"
        class="min-w-0 flex items-center gap-2 text-xs text-on-surface-secondary cursor-pointer select-none"
        :aria-expanded="!isCollapsed"
        @click="isCollapsed = !isCollapsed"
      >
        <div
          class="w-3 h-3 shrink-0 text-primary-500 transition-transform duration-200 motion-reduce:transition-none"
          :class="isCollapsed ? 'i-carbon-chevron-right' : 'i-carbon-chevron-down'"
        />
        <div class="i-carbon-list-checked w-3.5 h-3.5 shrink-0 text-teal-600 dark:text-teal-400" />
        <span class="truncate">执行计划</span>
        <span class="shrink-0 opacity-70">({{ completed }}/{{ total }})</span>
      </button>

      <n-tooltip v-if="readError || realtimeError">
        <template #trigger>
          <n-button text size="tiny" class="shrink-0 opacity-70 hover:opacity-100" @click="retry">
            <template #icon>
              <div class="i-carbon-renew w-3.5 h-3.5" />
            </template>
          </n-button>
        </template>
        重新读取执行计划
      </n-tooltip>
    </div>

    <div v-if="!isCollapsed" class="ml-1 pl-4 border-l border-gray-500/60 space-y-2" aria-live="polite">
      <div v-if="loading" class="min-h-8 flex items-center gap-2 text-xs text-on-surface-secondary">
        <div class="i-carbon-circle-dash w-3.5 h-3.5 animate-spin motion-reduce:animate-none" />
        <span>正在读取计划...</span>
      </div>

      <div v-else-if="readError" class="min-h-8 flex items-start gap-2 text-xs text-red-700 dark:text-red-300">
        <div class="i-carbon-warning-alt w-3.5 h-3.5 mt-0.5 shrink-0" />
        <div class="min-w-0 break-words [overflow-wrap:anywhere]">
          <div class="font-medium">
            计划读取失败
          </div>
          <div class="mt-0.5 opacity-70">
            {{ readError }}
          </div>
        </div>
      </div>

      <template v-else>
        <div v-if="realtimeError" class="flex items-start gap-2 text-xs text-yellow-700 dark:text-yellow-300">
          <div class="i-carbon-warning w-3.5 h-3.5 mt-0.5 shrink-0" />
          <div class="min-w-0 break-words [overflow-wrap:anywhere]">
            实时刷新不可用，可使用右侧按钮重新连接
          </div>
        </div>

        <div v-if="items.length === 0" class="min-h-8 flex items-center gap-2 text-xs text-on-surface-secondary">
          <div class="i-carbon-list-boxes w-3.5 h-3.5 shrink-0 opacity-70" />
          <span>暂无执行计划</span>
        </div>

        <template v-else>
          <div class="h-1 bg-container-tertiary rounded-sm overflow-hidden" role="progressbar" :aria-valuenow="progressPercent" aria-valuemin="0" aria-valuemax="100">
            <div
              class="h-full bg-teal-600 dark:bg-teal-400 transition-[width] duration-200 ease-out motion-reduce:transition-none"
              :style="{ width: `${progressPercent}%` }"
            />
          </div>

          <ol class="space-y-1.5">
            <li
              v-for="item in items"
              :key="item.id"
              class="plan-item min-w-0 flex items-start gap-2 py-0.5"
              :class="[
                item.status === 'completed' ? 'opacity-70' : '',
                completedAnimationId === item.id ? 'plan-item-completed-now' : '',
              ]"
            >
              <div
                class="plan-status-icon w-3.5 h-3.5 mt-0.5 shrink-0"
                :class="statusIcon(item.status)"
              />
              <div class="min-w-0 flex-1 flex flex-wrap items-baseline gap-x-2 gap-y-0.5">
                <span class="min-w-0 text-xs leading-5 text-on-surface break-words [overflow-wrap:anywhere]">
                  {{ item.text }}
                </span>
                <span class="shrink-0 text-[11px] leading-4" :class="statusLabelClass(item.status)">
                  {{ statusLabel(item.status) }}
                </span>
              </div>
            </li>
          </ol>

          <div v-if="allCompleted" class="flex items-center gap-2 text-xs text-green-700 dark:text-green-300">
            <div class="i-carbon-checkmark-outline w-3.5 h-3.5 shrink-0" />
            <span>计划已全部完成</span>
          </div>
        </template>
      </template>
    </div>
  </section>
</template>

<style scoped>
@keyframes plan-completed {
  0% { transform: scale(0.8); opacity: 0.4; }
  60% { transform: scale(1.15); opacity: 1; }
  100% { transform: scale(1); opacity: 1; }
}

.plan-item-completed-now .plan-status-icon {
  animation: plan-completed 220ms ease-out;
}

@media (prefers-reduced-motion: reduce) {
  .plan-item-completed-now .plan-status-icon {
    animation: none;
  }
}
</style>
