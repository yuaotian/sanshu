import type { UnlistenFn } from '@tauri-apps/api/event'
import type { PlanSnapshot } from '../types/plan'
import { invoke } from '@tauri-apps/api/core'
import { listen } from '@tauri-apps/api/event'
import { computed, onMounted, onUnmounted, ref, type Ref, watch } from 'vue'

interface UsePlanSessionOptions {
  workspace: Ref<string>
  adapter?: PlanSessionAdapter
}

export interface PlanSessionAdapter {
  getSnapshot: (workspace: string) => Promise<PlanSnapshot>
  startWatch: (workspace: string) => Promise<void>
  stopWatch: () => Promise<void>
  listen: (handler: () => void) => Promise<UnlistenFn>
}

const tauriPlanSessionAdapter: PlanSessionAdapter = {
  getSnapshot: workspace => invoke<PlanSnapshot>('get_plan_snapshot', { workspace }),
  startWatch: workspace => invoke('start_plan_watch', { workspace }),
  stopWatch: () => invoke('stop_plan_watch'),
  listen: handler => listen('plan-updated', handler),
}

/**
 * 只管理计划会话，不把 Tauri 监听和过期请求保护泄漏到展示模板。
 * 业务层仍使用原有三个命令和 plan-updated 事件。
 */
export function usePlanSession({ workspace, adapter = tauriPlanSessionAdapter }: UsePlanSessionOptions) {
  const snapshot = ref<PlanSnapshot | null>(null)
  const loading = ref(true)
  const readError = ref('')
  const watchError = ref('')
  const eventError = ref('')

  let mounted = false
  let loadSequence = 0
  let lifecycleGeneration = 0
  let watchGeneration = 0
  let unlistenPlanUpdate: UnlistenFn | null = null
  let listenerSetupPromise: Promise<boolean> | null = null
  let watchSetupQueue: Promise<void> = Promise.resolve()

  const items = computed(() => snapshot.value?.items ?? [])
  const completed = computed(() => snapshot.value?.summary.completed ?? 0)
  const total = computed(() => snapshot.value?.summary.total ?? 0)
  const allCompleted = computed(() => snapshot.value?.summary.all_completed ?? false)
  const progressPercent = computed(() => total.value === 0 ? 0 : Math.round((completed.value / total.value) * 100))
  const realtimeError = computed(() => eventError.value || watchError.value)

  function isCurrentWatch(generation: number): boolean {
    return mounted && generation === watchGeneration
  }

  function isCurrentLifecycle(generation: number): boolean {
    return mounted && generation === lifecycleGeneration
  }

  async function loadPlan(showLoading = false, requestedWorkspace = workspace.value): Promise<void> {
    const sequence = ++loadSequence
    if (showLoading)
      loading.value = true
    readError.value = ''

    try {
      const nextSnapshot = await adapter.getSnapshot(requestedWorkspace)
      if (sequence === loadSequence)
        snapshot.value = nextSnapshot
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

  async function stopWorkspaceWatch(): Promise<void> {
    try {
      await adapter.stopWatch()
    }
    catch (error) {
      console.warn('停止计划文件监听失败：', error)
    }
  }

  async function startWorkspaceWatch(generation: number, currentWorkspace: string): Promise<void> {
    if (!isCurrentWatch(generation))
      return

    watchError.value = ''
    snapshot.value = null
    loading.value = true

    let started = false
    try {
      await adapter.startWatch(currentWorkspace)
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
    void loadPlan(true, currentWorkspace)
  }

  function queueWorkspaceWatch(generation: number, currentWorkspace: string): Promise<void> {
    // 中文说明：串行建立 watcher，过期任务完成后先清理，再启动最新工作区。
    watchSetupQueue = watchSetupQueue.then(() => startWorkspaceWatch(generation, currentWorkspace))
    return watchSetupQueue
  }

  async function ensurePlanListener(): Promise<boolean> {
    if (unlistenPlanUpdate)
      return mounted
    if (listenerSetupPromise)
      return listenerSetupPromise

    const generation = lifecycleGeneration
    const setupPromise = (async () => {
      try {
        const unlisten = await adapter.listen(() => {
          if (mounted)
            void loadPlan()
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

  async function restartWorkspaceWatch(): Promise<void> {
    const generation = ++watchGeneration
    const currentWorkspace = workspace.value
    loadSequence += 1

    if (await ensurePlanListener() && isCurrentWatch(generation))
      await queueWorkspaceWatch(generation, currentWorkspace)
  }

  async function retry(): Promise<void> {
    await restartWorkspaceWatch()
  }

  watch(workspace, async (currentWorkspace, previousWorkspace) => {
    if (mounted && currentWorkspace !== previousWorkspace)
      await restartWorkspaceWatch()
  })

  onMounted(async () => {
    mounted = true
    await restartWorkspaceWatch()
  })

  onUnmounted(() => {
    mounted = false
    lifecycleGeneration += 1
    watchGeneration += 1
    loadSequence += 1
    unlistenPlanUpdate?.()
    unlistenPlanUpdate = null
    void stopWorkspaceWatch()
  })

  return {
    allCompleted,
    completed,
    eventError,
    items,
    loading,
    loadPlan,
    progressPercent,
    readError,
    realtimeError,
    retry,
    snapshot,
    total,
    watchError,
  }
}
