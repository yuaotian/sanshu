import { onClickOutside, useEventListener, useResizeObserver, useStorage } from '@vueuse/core'
import { computed, nextTick, onMounted, onUnmounted, ref, type Ref, watch } from 'vue'

interface FloatingPosition {
  left: number
  top: number
}

interface UsePlanPanelPreferencesOptions {
  workspace: Ref<string>
  panelRef: Ref<HTMLElement | null>
}

function clamp(value: number, min: number, max: number): number {
  return Math.min(Math.max(value, min), Math.max(min, max))
}

/**
 * 管理面板自身的工作区偏好和悬浮交互，避免 localStorage/Pointer Capture 与计划会话耦合。
 */
export function usePlanPanelPreferences({ workspace, panelRef }: UsePlanPanelPreferencesOptions) {
  const isCollapsed = useStorage('popup-plan-panel-collapsed', true)
  const localNote = ref('')
  const noteDraft = ref('')
  const showAllItems = ref(false)
  const settingsOpen = ref(false)
  const isFloating = ref(false)
  const floatingPosition = ref<FloatingPosition | null>(null)
  const isDragging = ref(false)

  let preferencesReady = false
  let dragPointerId: number | null = null
  let dragOffset = { x: 0, y: 0 }

  function storageKey(kind: string, currentWorkspace = workspace.value): string {
    return `popup-plan-panel-${kind}:${encodeURIComponent(currentWorkspace)}`
  }

  function readStoredValue(key: string): string | null {
    try {
      return localStorage.getItem(key)
    }
    catch {
      return null
    }
  }

  function writeStoredValue(key: string, value: string): void {
    try {
      localStorage.setItem(key, value)
    }
    catch {
      // 中文说明：本地存储不可用时保留内存态，不阻断计划面板的核心读取。
    }
  }

  function removeStoredValue(key: string): void {
    try {
      localStorage.removeItem(key)
    }
    catch {
      // 中文说明：清理偏好失败不影响计划状态展示。
    }
  }

  function parseFloatingPosition(value: string | null): FloatingPosition | null {
    if (!value)
      return null
    try {
      const parsed = JSON.parse(value) as Partial<FloatingPosition>
      if (typeof parsed.left === 'number' && Number.isFinite(parsed.left) && typeof parsed.top === 'number' && Number.isFinite(parsed.top))
        return { left: parsed.left, top: parsed.top }
    }
    catch {
      // 中文说明：坐标损坏时回退到窗口右下角默认位置。
    }
    return null
  }

  function loadPanelPreferences(currentWorkspace = workspace.value): void {
    // 中文说明：备注、悬浮开关和坐标按工作区隔离，避免不同项目互相覆盖偏好。
    preferencesReady = false
    localNote.value = readStoredValue(storageKey('note', currentWorkspace)) ?? ''
    noteDraft.value = ''
    isFloating.value = readStoredValue(storageKey('floating', currentWorkspace)) === 'true'
    floatingPosition.value = parseFloatingPosition(readStoredValue(storageKey('position', currentWorkspace)))
    showAllItems.value = false
    settingsOpen.value = false
    preferencesReady = true
  }

  function persistFloatingPreferences(): void {
    if (!preferencesReady)
      return
    writeStoredValue(storageKey('floating'), String(isFloating.value))
    if (floatingPosition.value)
      writeStoredValue(storageKey('position'), JSON.stringify(floatingPosition.value))
    else
      removeStoredValue(storageKey('position'))
  }

  function saveLocalNote(): void {
    const nextNote = noteDraft.value.trim()
    if (!nextNote)
      return
    localNote.value = nextNote
    noteDraft.value = ''
    writeStoredValue(storageKey('note'), nextNote)
  }

  function clearLocalNote(): void {
    localNote.value = ''
    removeStoredValue(storageKey('note'))
  }

  function clampFloatingPosition(shouldPersist = false): void {
    if (!isFloating.value || !floatingPosition.value || !panelRef.value || typeof window === 'undefined')
      return

    const rect = panelRef.value.getBoundingClientRect()
    const margin = 12
    const nextPosition = {
      left: clamp(floatingPosition.value.left, margin, window.innerWidth - rect.width - margin),
      top: clamp(floatingPosition.value.top, margin, window.innerHeight - rect.height - margin),
    }
    if (nextPosition.left === floatingPosition.value.left && nextPosition.top === floatingPosition.value.top)
      return
    floatingPosition.value = nextPosition
    if (shouldPersist)
      persistFloatingPreferences()
  }

  function ensureDefaultFloatingPosition(): void {
    if (!isFloating.value || !panelRef.value || typeof window === 'undefined')
      return

    if (!floatingPosition.value) {
      const rect = panelRef.value.getBoundingClientRect()
      floatingPosition.value = {
        left: Math.max(12, window.innerWidth - rect.width - 16),
        top: Math.max(12, window.innerHeight - rect.height - 16),
      }
      persistFloatingPreferences()
    }
    else {
      clampFloatingPosition(true)
    }
  }

  function startDragging(event: PointerEvent): void {
    // 中文说明：仅允许从明确的拖动手柄开始，避免抢占标题折叠和正文选中文本。
    if (!isFloating.value || event.button !== 0 || !panelRef.value)
      return
    const rect = panelRef.value.getBoundingClientRect()
    dragPointerId = event.pointerId
    dragOffset = { x: event.clientX - rect.left, y: event.clientY - rect.top }
    isDragging.value = true
    panelRef.value.setPointerCapture?.(event.pointerId)
    event.preventDefault()
  }

  function handleDragging(event: PointerEvent): void {
    if (!isDragging.value || dragPointerId !== event.pointerId || !panelRef.value || typeof window === 'undefined')
      return
    const rect = panelRef.value.getBoundingClientRect()
    const margin = 12
    floatingPosition.value = {
      left: clamp(event.clientX - dragOffset.x, margin, window.innerWidth - rect.width - margin),
      top: clamp(event.clientY - dragOffset.y, margin, window.innerHeight - rect.height - margin),
    }
  }

  function stopDragging(event?: PointerEvent): void {
    if (!isDragging.value)
      return
    if (event && dragPointerId !== event.pointerId)
      return
    if (event)
      panelRef.value?.releasePointerCapture?.(event.pointerId)
    dragPointerId = null
    isDragging.value = false
    persistFloatingPreferences()
  }

  const floatingStyle = computed(() => {
    if (!isFloating.value || !floatingPosition.value)
      return undefined
    return {
      left: `${floatingPosition.value.left}px`,
      top: `${floatingPosition.value.top}px`,
      right: 'auto',
      bottom: 'auto',
    }
  })

  watch(isFloating, (enabled) => {
    if (!preferencesReady)
      return
    if (enabled)
      nextTick(ensureDefaultFloatingPosition)
    persistFloatingPreferences()
  }, { flush: 'post' })

  watch(workspace, (currentWorkspace, previousWorkspace) => {
    if (currentWorkspace !== previousWorkspace)
      loadPanelPreferences(currentWorkspace)
  })

  useResizeObserver(panelRef, () => {
    if (isFloating.value)
      clampFloatingPosition(true)
  })
  useEventListener('resize', () => clampFloatingPosition(true))
  onClickOutside(panelRef, () => {
    settingsOpen.value = false
  })

  onMounted(() => {
    loadPanelPreferences()
    nextTick(ensureDefaultFloatingPosition)
  })

  onUnmounted(() => {
    stopDragging()
  })

  return {
    clearLocalNote,
    floatingPosition,
    floatingStyle,
    handleDragging,
    isCollapsed,
    isDragging,
    isFloating,
    localNote,
    loadPanelPreferences,
    noteDraft,
    saveLocalNote,
    settingsOpen,
    showAllItems,
    startDragging,
    stopDragging,
  }
}
