<script setup lang="ts">
import { invoke } from '@tauri-apps/api/core'
import { useMessage } from 'naive-ui'
import { computed, nextTick, onBeforeUnmount, onMounted, ref, watch } from 'vue'
import { useIconSearch } from '../../../composables/useIconSearch'
import type { IconFormat, IconItem, IconSaveItem, IconSaveRequest, IconSaveResult } from '../../../types/icon'
import IconWorkshop from './IconWorkshop.vue'

interface Props {
  initialQuery?: string
  initialStyle?: string
  initialSavePath?: string
  projectRoot?: string
}

const props = defineProps<Props>()

const message = useMessage()
const { saveIcons } = useIconSearch()

// ============ 保存进度状态 ============
const isSaving = ref(false)
const saveProgress = ref(0)
const savingIconName = ref('')
const saveSummary = ref<IconSaveResult | null>(null)
const saveError = ref<string | null>(null)
const needsConfirm = ref(false)
const pendingResponse = ref<any>(null)

// ============ 选择与编辑状态 ============
const selectedIcons = ref<IconItem[]>([])
const activeIconId = ref<number | null>(null)
const editorStatus = ref<'idle' | 'loading' | 'ready' | 'error'>('idle')
const editorPreviewSvg = ref<string>('')
const editorSavePath = ref(props.initialSavePath || 'assets/icons')
const recentColors = ref<string[]>([])

const paletteColors = [
  '#C6B8A6',
  '#B9B2A1',
  '#A9B7B0',
  '#B5C1C9',
  '#C1B4C3',
  '#C7B198',
  '#B9C2A5',
  '#AEBACB',
  '#C3B7A6',
  '#B6B6B6',
]

const sizePresets = [16, 24, 32, 48, 64, 96, 128]
const pngSizePresets = [16, 32, 64, 128, 258]
const previewScaleOptions = [
  { label: '50%', value: 50 },
  { label: '75%', value: 75 },
  { label: '100%', value: 100 },
  { label: '125%', value: 125 },
  { label: '150%', value: 150 },
  { label: '200%', value: 200 },
]

interface IconEditorState {
  color: string
  applyColor: boolean
  width: number
  height: number
  rotate: number
  flipX: boolean
  flipY: boolean
  roundStroke: boolean
  strokeWidth: number | null
  rectRadius: number | null
  // 线条级别编辑
  activeElementKey: string | null
  elementStyles: Record<string, SvgElementStyle>
}

interface SvgElementOption {
  key: string
  label: string
  tag: string
}

interface SvgElementStyle {
  enabled: boolean
  strokeColor: string
  strokeWidth: number | null
  roundStroke: boolean
}

const editorStates = ref<Record<number, IconEditorState>>({})
const originalSvgMap = ref<Record<number, string>>({})
const editedSvgMap = ref<Record<number, string>>({})
const elementOptionsMap = ref<Record<number, SvgElementOption[]>>({})
const elementDefaultStyles = ref<Record<number, Record<string, SvgElementStyle>>>({})

// ============ 编辑器弹窗状态 ============
const editorModalOpen = ref(false)
const editorModalRef = ref<HTMLElement | null>(null)
const editorInteracting = ref(false)
const previewBackground = ref<'grid' | 'light' | 'dark'>('grid')
const previewScale = ref(100)
const previewUpdating = ref(false)
const elementSearch = ref('')
const editorSaveFormat = ref<IconFormat>('svg')
const editorPngSize = ref(128)
const editorCollapseExpanded = ref(['appearance', 'transform'])

// ============ 右键菜单状态 ============
const contextMenuVisible = ref(false)
const contextMenuPosition = ref({ x: 0, y: 0 })
const contextMenuIcon = ref<IconItem | null>(null)
const editorRect = ref({ x: 0, y: 0, width: 0, height: 0 })
const editorRectInitialized = ref(false)
const dragState = ref<{
  mode: 'move' | 'resize' | null
  startX: number
  startY: number
  startLeft: number
  startTop: number
  startWidth: number
  startHeight: number
  prevUserSelect: string
}>({
  mode: null,
  startX: 0,
  startY: 0,
  startLeft: 0,
  startTop: 0,
  startWidth: 0,
  startHeight: 0,
  prevUserSelect: '',
})

const activeIcon = computed(() => {
  if (!selectedIcons.value.length)
    return null
  return selectedIcons.value.find(icon => icon.id === activeIconId.value) || selectedIcons.value[0] || null
})

const activeState = ref<IconEditorState | null>(null)
const activeElementOptions = computed(() => {
  const icon = activeIcon.value
  if (!icon)
    return []
  return elementOptionsMap.value[icon.id] || []
})
const activeElementKey = computed({
  get: () => activeState.value?.activeElementKey ?? null,
  set: (value) => {
    if (activeState.value)
      activeState.value.activeElementKey = value
  },
})
const activeElementStyle = computed(() => {
  const state = activeState.value
  if (!state || !state.activeElementKey)
    return null
  return state.elementStyles[state.activeElementKey] || null
})

const filteredElementOptions = computed(() => {
  const options = activeElementOptions.value
  const keyword = elementSearch.value.trim().toLowerCase()
  if (!keyword)
    return options
  const filtered = options.filter(option => option.label.toLowerCase().includes(keyword) || option.tag.toLowerCase().includes(keyword))
  if (activeElementKey.value) {
    const current = options.find(option => option.key === activeElementKey.value)
    if (current && !filtered.some(option => option.key === current.key))
      filtered.unshift(current)
  }
  return filtered
})

const mergedSwatches = computed(() => {
  const list = [...recentColors.value, ...paletteColors]
  const seen = new Set<string>()
  return list.filter((color) => {
    const key = color.toUpperCase()
    if (seen.has(key))
      return false
    seen.add(key)
    return true
  }).slice(0, 12)
})

const previewBackgroundClass = computed(() => {
  if (previewBackground.value === 'light')
    return 'bg-stone-50'
  if (previewBackground.value === 'dark')
    return 'bg-slate-900'
  return 'bg-slate-900'
})

const previewScaleClass = computed(() => {
  const map: Record<number, string> = {
    50: 'scale-50',
    75: 'scale-75',
    100: 'scale-100',
    125: 'scale-125',
    150: 'scale-150',
    200: 'scale-200',
  }
  return map[previewScale.value] || 'scale-100'
})

const previewBusy = computed(() => editorStatus.value === 'loading' || previewUpdating.value)

const needsPngSize = computed(() => editorSaveFormat.value === 'png' || editorSaveFormat.value === 'both')

const editorStatusLabel = computed(() => {
  if (editorStatus.value === 'loading')
    return '加载中'
  if (editorStatus.value === 'ready')
    return '就绪'
  if (editorStatus.value === 'error')
    return '加载失败'
  return '未选择'
})

const editorStatusTagType = computed(() => {
  if (editorStatus.value === 'loading')
    return 'warning'
  if (editorStatus.value === 'ready')
    return 'success'
  if (editorStatus.value === 'error')
    return 'error'
  return 'default'
})

const showProgressOverlay = computed(() => isSaving.value || needsConfirm.value)

watch(() => props.initialSavePath, (value) => {
  if (value)
    editorSavePath.value = value
})

watch(selectedIcons, (icons) => {
  if (!icons.length) {
    activeIconId.value = null
    editorPreviewSvg.value = ''
    editorStatus.value = 'idle'
    return
  }

  if (!activeIconId.value || !icons.some(icon => icon.id === activeIconId.value))
    activeIconId.value = icons[0].id
})

watch(activeIcon, async (icon) => {
  if (!icon) {
    editorStatus.value = 'idle'
    editorPreviewSvg.value = ''
    activeState.value = null
    return
  }
  elementSearch.value = ''
  await prepareEditor(icon)
})

// 防抖更新预览
let previewTimer: number | null = null
watch(activeState, () => {
  schedulePreviewUpdate()
}, { deep: true })

watch(activeElementKey, () => {
  ensureActiveElementStyle()
}, { immediate: true })

watch(() => [activeState.value?.color, activeState.value?.applyColor], ([color, apply]) => {
  if (apply && color && typeof color === 'string')
    pushRecentColor(color)
})

watch(editorRect, () => {
  applyEditorRect()
}, { deep: true })

watch(editorModalOpen, (open) => {
  if (!open)
    return
  nextTick(() => {
    initEditorRect()
    clampEditorRect()
    applyEditorRect()
  })
})

onMounted(() => {
  window.addEventListener('resize', handleWindowResize)
})

onBeforeUnmount(() => {
  window.removeEventListener('resize', handleWindowResize)
  handlePointerUp()
})

function handleSelectionChange(icons: IconItem[]) {
  selectedIcons.value = icons
}

// ============ 双击与右键菜单处理 ============

// 双击图标：选中并打开编辑器
function handleIconDblClick(icon: IconItem) {
  // 确保图标被选中
  if (!selectedIcons.value.some(i => i.id === icon.id)) {
    selectedIcons.value = [...selectedIcons.value, icon]
  }
  // 设置为当前活动图标
  activeIconId.value = icon.id
  // 打开编辑器
  openEditorModal()
}

// 右键图标：显示上下文菜单
function handleIconContextMenu(icon: IconItem, event: MouseEvent) {
  contextMenuIcon.value = icon
  contextMenuPosition.value = { x: event.clientX, y: event.clientY }
  contextMenuVisible.value = true
}

// 关闭右键菜单
function closeContextMenu() {
  contextMenuVisible.value = false
  contextMenuIcon.value = null
}

// 右键菜单：打开编辑器
function contextMenuOpenEditor() {
  if (!contextMenuIcon.value)
    return
  handleIconDblClick(contextMenuIcon.value)
  closeContextMenu()
}

// 右键菜单：复制SVG
async function contextMenuCopySvg() {
  if (!contextMenuIcon.value)
    return
  const icon = contextMenuIcon.value
  try {
    // 获取编辑后的SVG或原始SVG
    const svgContent = getEditedSvg(icon) || icon.svgContent
    if (svgContent) {
      await navigator.clipboard.writeText(svgContent)
      message.success(`已复制 ${icon.name} 的 SVG`)
    }
    else {
      message.warning('暂无可复制的 SVG 内容')
    }
  }
  catch (error) {
    console.error('复制失败:', error)
    message.error('复制失败')
  }
  closeContextMenu()
}

function schedulePreviewUpdate() {
  if (previewTimer)
    window.clearTimeout(previewTimer)

  const icon = activeIcon.value
  const state = activeState.value
  if (!icon || !state) {
    previewUpdating.value = false
    return
  }

  previewUpdating.value = true
  previewTimer = window.setTimeout(() => {
    const originalSvg = originalSvgMap.value[icon.id]
    if (!originalSvg) {
      previewUpdating.value = false
      return
    }

    const { finalSvg, previewSvg } = buildEditedSvgPair(originalSvg, state, state.activeElementKey)
    editorPreviewSvg.value = previewSvg
    editedSvgMap.value[icon.id] = finalSvg
    previewUpdating.value = false
  }, 200)
}

async function prepareEditor(icon: IconItem) {
  editorStatus.value = 'loading'

  const originalSvg = await ensureOriginalSvg(icon)
  if (!originalSvg) {
    editorStatus.value = 'error'
    previewUpdating.value = false
    return
  }

  ensureEditableElements(icon, originalSvg)

  if (!editorStates.value[icon.id])
    editorStates.value[icon.id] = createDefaultState(originalSvg)

  activeState.value = editorStates.value[icon.id]
  const options = elementOptionsMap.value[icon.id] || []
  if (activeState.value.activeElementKey && !options.some(option => option.key === activeState.value?.activeElementKey)) {
    activeState.value.activeElementKey = null
  }
  editorStatus.value = 'ready'
  previewUpdating.value = false
  ensureActiveElementStyle()
  schedulePreviewUpdate()
}

async function ensureOriginalSvg(icon: IconItem) {
  if (originalSvgMap.value[icon.id])
    return originalSvgMap.value[icon.id]

  if (icon.svgContent) {
    originalSvgMap.value[icon.id] = icon.svgContent
    return icon.svgContent
  }

  try {
    // 从后端获取 SVG 内容（兜底）
    const result = await invoke<any>('get_icon_content', {
      request: { id: icon.id, format: 'svg' },
    })
    if (result?.svg_content) {
      originalSvgMap.value[icon.id] = result.svg_content
      return result.svg_content
    }
  }
  catch (error) {
    console.error('获取 SVG 内容失败:', error)
  }

  return null
}

function ensureEditableElements(icon: IconItem, svg: string) {
  if (elementOptionsMap.value[icon.id])
    return
  const { options, defaults } = extractEditableElements(svg)
  elementOptionsMap.value[icon.id] = options
  elementDefaultStyles.value[icon.id] = defaults
}

function extractEditableElements(svg: string) {
  try {
    const doc = new DOMParser().parseFromString(svg, 'image/svg+xml')
    const svgEl = doc.querySelector('svg')
    if (!svgEl)
      return { options: [], defaults: {} as Record<string, SvgElementStyle> }

    const nodes = collectEditableNodes(svgEl)
    const options: SvgElementOption[] = []
    const defaults: Record<string, SvgElementStyle> = {}

    nodes.forEach((node, index) => {
      const key = buildElementKey(node, index)
      const tag = node.tagName.toLowerCase()
      const id = node.getAttribute('id') || ''
      const label = id ? `${tag}#${id}` : `${tag} ${index + 1}`
      options.push({ key, label, tag })
      defaults[key] = createElementStyleFromNode(node)
    })

    return { options, defaults }
  }
  catch (error) {
    console.error('解析 SVG 线条元素失败:', error)
    return { options: [], defaults: {} as Record<string, SvgElementStyle> }
  }
}

function collectEditableNodes(svgEl: SVGSVGElement) {
  const nodes = Array.from(svgEl.querySelectorAll('path, line, rect, circle, ellipse, polyline, polygon'))
  return nodes.filter(node => !node.closest('defs'))
}

function buildElementKey(node: Element, index: number) {
  return `${node.tagName.toLowerCase()}-${index + 1}`
}

function createElementStyleFromNode(node: Element): SvgElementStyle {
  const stroke = node.getAttribute('stroke')
  const strokeWidth = node.getAttribute('stroke-width')
  const linecap = node.getAttribute('stroke-linecap')
  const linejoin = node.getAttribute('stroke-linejoin')
  const parsedWidth = strokeWidth ? Number.parseFloat(strokeWidth) : null

  return {
    enabled: Boolean(stroke || strokeWidth || linecap || linejoin),
    strokeColor: stroke || '#8B8B8B',
    strokeWidth: Number.isFinite(parsedWidth) ? parsedWidth : null,
    roundStroke: linecap === 'round' || linejoin === 'round',
  }
}

function ensureActiveElementStyle() {
  const icon = activeIcon.value
  const state = activeState.value
  if (!icon || !state || !state.activeElementKey)
    return

  if (!state.elementStyles[state.activeElementKey]) {
    const defaults = elementDefaultStyles.value[icon.id]?.[state.activeElementKey]
    state.elementStyles[state.activeElementKey] = defaults
      ? { ...defaults }
      : {
          enabled: false,
          strokeColor: '#8B8B8B',
          strokeWidth: null,
          roundStroke: false,
        }
  }
}

function updateActiveElementStyle<K extends keyof SvgElementStyle>(key: K, value: SvgElementStyle[K]) {
  const style = activeElementStyle.value
  if (!style)
    return
  style[key] = value
  schedulePreviewUpdate()
}

function resetActiveElementStyle() {
  const icon = activeIcon.value
  const state = activeState.value
  if (!icon || !state || !state.activeElementKey)
    return
  const defaults = elementDefaultStyles.value[icon.id]?.[state.activeElementKey]
  state.elementStyles[state.activeElementKey] = defaults
    ? { ...defaults }
    : {
        enabled: false,
        strokeColor: '#8B8B8B',
        strokeWidth: null,
        roundStroke: false,
      }
  schedulePreviewUpdate()
}

function createDefaultState(svg: string): IconEditorState {
  const { width, height } = parseSvgSize(svg)
  return {
    color: '#8B8B8B',
    applyColor: false,
    width,
    height,
    rotate: 0,
    flipX: false,
    flipY: false,
    roundStroke: false,
    strokeWidth: null,
    rectRadius: null,
    activeElementKey: null,
    elementStyles: {},
  }
}

function parseSvgSize(svg: string) {
  try {
    const doc = new DOMParser().parseFromString(svg, 'image/svg+xml')
    const svgEl = doc.querySelector('svg')
    if (!svgEl)
      return { width: 64, height: 64 }

    const widthAttr = svgEl.getAttribute('width')
    const heightAttr = svgEl.getAttribute('height')
    const viewBox = svgEl.getAttribute('viewBox')

    const width = widthAttr ? Number.parseFloat(widthAttr) : Number.NaN
    const height = heightAttr ? Number.parseFloat(heightAttr) : Number.NaN

    if (Number.isFinite(width) && Number.isFinite(height))
      return { width, height }

    if (viewBox) {
      const parts = viewBox.split(/\s+/).map(v => Number.parseFloat(v))
      if (parts.length === 4 && parts.every(v => Number.isFinite(v))) {
        return { width: parts[2], height: parts[3] }
      }
    }
  }
  catch (error) {
    console.error('解析 SVG 尺寸失败:', error)
  }

  return { width: 64, height: 64 }
}

function buildEditedSvgPair(svg: string, state: IconEditorState, focusKey: string | null) {
  try {
    const doc = new DOMParser().parseFromString(svg, 'image/svg+xml')
    const svgEl = doc.querySelector('svg')
    if (!svgEl)
      return { finalSvg: svg, previewSvg: svg }

    // 按当前编辑状态应用颜色、尺寸与变换
    const viewBoxInfo = readViewBox(svgEl, state)

    svgEl.setAttribute('width', String(state.width))
    svgEl.setAttribute('height', String(state.height))

    if (state.applyColor) {
      svgEl.setAttribute('fill', state.color)
      svgEl.setAttribute('stroke', state.color)
    }

    if (state.roundStroke) {
      svgEl.setAttribute('stroke-linecap', 'round')
      svgEl.setAttribute('stroke-linejoin', 'round')
    }
    else {
      svgEl.removeAttribute('stroke-linecap')
      svgEl.removeAttribute('stroke-linejoin')
    }

    if (state.strokeWidth !== null) {
      if (state.strokeWidth > 0)
        svgEl.setAttribute('stroke-width', String(state.strokeWidth))
      else
        svgEl.removeAttribute('stroke-width')
    }

    if (state.rectRadius !== null) {
      const rects = svgEl.querySelectorAll('rect')
      rects.forEach((rect) => {
        rect.setAttribute('rx', String(Math.max(0, state.rectRadius as number)))
        rect.setAttribute('ry', String(Math.max(0, state.rectRadius as number)))
      })
    }

    // 应用线条级别覆盖
    const editableNodes = collectEditableNodes(svgEl)
    let focusNode: Element | null = null
    editableNodes.forEach((node, index) => {
      const key = buildElementKey(node, index)
      if (focusKey && key === focusKey)
        focusNode = node

      const style = state.elementStyles[key]
      if (!style || !style.enabled)
        return

      if (style.strokeColor)
        node.setAttribute('stroke', style.strokeColor)

      if (style.strokeWidth !== null) {
        if (style.strokeWidth > 0)
          node.setAttribute('stroke-width', String(style.strokeWidth))
        else
          node.removeAttribute('stroke-width')
      }

      if (style.roundStroke) {
        node.setAttribute('stroke-linecap', 'round')
        node.setAttribute('stroke-linejoin', 'round')
      }
      else {
        node.removeAttribute('stroke-linecap')
        node.removeAttribute('stroke-linejoin')
      }
    })

    const transforms: string[] = []
    if (state.flipX)
      transforms.push(`translate(${viewBoxInfo.minX * 2 + viewBoxInfo.width} 0) scale(-1 1)`)
    if (state.flipY)
      transforms.push(`translate(0 ${viewBoxInfo.minY * 2 + viewBoxInfo.height}) scale(1 -1)`)
    if (state.rotate)
      transforms.push(`rotate(${state.rotate} ${viewBoxInfo.minX + viewBoxInfo.width / 2} ${viewBoxInfo.minY + viewBoxInfo.height / 2})`)

    if (transforms.length) {
      const group = doc.createElementNS('http://www.w3.org/2000/svg', 'g')
      group.setAttribute('transform', transforms.join(' '))

      const children = Array.from(svgEl.childNodes)
      children.forEach((node) => {
        if (node.nodeType === 1 && (node as Element).tagName.toLowerCase() === 'defs')
          return
        group.appendChild(node)
      })

      svgEl.appendChild(group)
    }

    const serializer = new XMLSerializer()
    const finalSvg = serializer.serializeToString(svgEl)
    let previewSvg = finalSvg

    // 仅在预览中标记选中线条
    if (focusNode !== null) {
      (focusNode as Element).setAttribute('data-editor-focus', 'true')
      previewSvg = serializer.serializeToString(svgEl)
    }

    return { finalSvg, previewSvg }
  }
  catch (error) {
    console.error('应用 SVG 编辑失败:', error)
    return { finalSvg: svg, previewSvg: svg }
  }
}

function readViewBox(svgEl: SVGSVGElement, state: IconEditorState) {
  const viewBox = svgEl.getAttribute('viewBox')
  if (viewBox) {
    const parts = viewBox.split(/\s+/).map(v => Number.parseFloat(v))
    if (parts.length === 4 && parts.every(v => Number.isFinite(v))) {
      return {
        minX: parts[0],
        minY: parts[1],
        width: parts[2],
        height: parts[3],
      }
    }
  }

  return {
    minX: 0,
    minY: 0,
    width: state.width || 64,
    height: state.height || 64,
  }
}

function pushRecentColor(color: string) {
  const normalized = color.toUpperCase()
  const list = recentColors.value.filter(item => item.toUpperCase() !== normalized)
  list.unshift(color)
  recentColors.value = list.slice(0, 6)
}

function applySizePreset(size: number) {
  updateActiveState('width', size)
  updateActiveState('height', size)
}

function updateEditorPngSize(value: number | null) {
  if (value === null)
    return
  editorPngSize.value = value
}

function applyPngSizePreset(size: number) {
  editorPngSize.value = size
}

function updateActiveState<K extends keyof IconEditorState>(key: K, value: IconEditorState[K]) {
  const state = activeState.value
  if (!state)
    return
  state[key] = value
}

function toggleActiveState(key: 'flipX' | 'flipY') {
  const state = activeState.value
  if (!state)
    return
  state[key] = !state[key]
}

function resetActiveEditor() {
  const icon = activeIcon.value
  if (!icon)
    return
  const originalSvg = originalSvgMap.value[icon.id]
  if (!originalSvg)
    return
  editorStates.value[icon.id] = createDefaultState(originalSvg)
  activeState.value = editorStates.value[icon.id]
  schedulePreviewUpdate()
}

function openEditorModal() {
  editorModalOpen.value = true
  nextTick(() => {
    initEditorRect()
    clampEditorRect()
    applyEditorRect()
  })
}

function closeEditorModal() {
  editorModalOpen.value = false
  handlePointerUp()
}

function initEditorRect() {
  if (editorRectInitialized.value)
    return
  const { width, height } = getViewportSize()
  const baseWidth = Math.round(width * 0.58)
  const baseHeight = Math.round(height * 0.78)
  const targetWidth = Math.min(Math.max(baseWidth, 360), Math.max(360, width - 32))
  const targetHeight = Math.min(Math.max(baseHeight, 460), Math.max(460, height - 32))
  editorRect.value = {
    x: Math.round((width - targetWidth) / 2),
    y: Math.round((height - targetHeight) / 2),
    width: targetWidth,
    height: targetHeight,
  }
  editorRectInitialized.value = true
}

function getViewportSize() {
  return { width: window.innerWidth, height: window.innerHeight }
}

function handleWindowResize() {
  if (!editorModalOpen.value)
    return
  clampEditorRect()
  applyEditorRect()
}

function clampEditorRect() {
  const { width, height } = getViewportSize()
  const padding = 12
  const maxWidth = Math.max(260, width - padding * 2)
  const maxHeight = Math.max(320, height - padding * 2)
  const minWidth = Math.min(360, maxWidth)
  const minHeight = Math.min(420, maxHeight)

  const nextWidth = Math.min(Math.max(editorRect.value.width, minWidth), maxWidth)
  const nextHeight = Math.min(Math.max(editorRect.value.height, minHeight), maxHeight)
  const nextX = Math.min(Math.max(editorRect.value.x, padding), width - nextWidth - padding)
  const nextY = Math.min(Math.max(editorRect.value.y, padding), height - nextHeight - padding)

  editorRect.value = {
    x: Math.max(padding, nextX),
    y: Math.max(padding, nextY),
    width: nextWidth,
    height: nextHeight,
  }
}

function applyEditorRect() {
  const el = editorModalRef.value
  if (!el)
    return
  el.style.transform = `translate(${editorRect.value.x}px, ${editorRect.value.y}px)`
  el.style.width = `${editorRect.value.width}px`
  el.style.height = `${editorRect.value.height}px`
}

function startDrag(event: PointerEvent) {
  if (!editorModalRef.value)
    return
  editorInteracting.value = true
  dragState.value = {
    mode: 'move',
    startX: event.clientX,
    startY: event.clientY,
    startLeft: editorRect.value.x,
    startTop: editorRect.value.y,
    startWidth: editorRect.value.width,
    startHeight: editorRect.value.height,
    prevUserSelect: document.body.style.userSelect || '',
  }
  document.body.style.userSelect = 'none'
  window.addEventListener('pointermove', handlePointerMove)
  window.addEventListener('pointerup', handlePointerUp)
}

function startResize(event: PointerEvent) {
  if (!editorModalRef.value)
    return
  editorInteracting.value = true
  dragState.value = {
    mode: 'resize',
    startX: event.clientX,
    startY: event.clientY,
    startLeft: editorRect.value.x,
    startTop: editorRect.value.y,
    startWidth: editorRect.value.width,
    startHeight: editorRect.value.height,
    prevUserSelect: document.body.style.userSelect || '',
  }
  document.body.style.userSelect = 'none'
  window.addEventListener('pointermove', handlePointerMove)
  window.addEventListener('pointerup', handlePointerUp)
}

function handlePointerMove(event: PointerEvent) {
  if (!dragState.value.mode)
    return
  const deltaX = event.clientX - dragState.value.startX
  const deltaY = event.clientY - dragState.value.startY

  if (dragState.value.mode === 'move') {
    editorRect.value.x = dragState.value.startLeft + deltaX
    editorRect.value.y = dragState.value.startTop + deltaY
  }
  else if (dragState.value.mode === 'resize') {
    editorRect.value.width = dragState.value.startWidth + deltaX
    editorRect.value.height = dragState.value.startHeight + deltaY
  }

  clampEditorRect()
  applyEditorRect()
}

function handlePointerUp() {
  dragState.value.mode = null
  editorInteracting.value = false
  document.body.style.userSelect = dragState.value.prevUserSelect
  window.removeEventListener('pointermove', handlePointerMove)
  window.removeEventListener('pointerup', handlePointerUp)
}

async function copyEditedSvg() {
  const icon = activeIcon.value
  const edited = icon ? getEditedSvg(icon) : editorPreviewSvg.value
  if (!edited) {
    message.warning('暂无可复制的 SVG')
    return
  }
  try {
    await navigator.clipboard.writeText(edited)
    message.success('已复制编辑后的 SVG')
  }
  catch (error) {
    console.error('复制 SVG 失败:', error)
    message.error('复制失败，请稍后重试')
  }
}

async function selectEditorDirectory() {
  try {
    const result = await invoke<string | null>('select_icon_save_directory', {
      defaultPath: editorSavePath.value,
    })
    if (result)
      editorSavePath.value = result
  }
  catch (error) {
    console.error('选择目录失败:', error)
    message.error('选择目录失败')
  }
}

function buildCustomIconName(name: string) {
  const now = new Date()
  const timestamp = `${now.getFullYear()}${String(now.getMonth() + 1).padStart(2, '0')}${String(now.getDate()).padStart(2, '0')}`
    + `${String(now.getHours()).padStart(2, '0')}${String(now.getMinutes()).padStart(2, '0')}${String(now.getSeconds()).padStart(2, '0')}`
  const random = Math.random().toString(36).slice(2, 6)
  return `${name}-${timestamp}-${random}`
}

function getEditedSvg(icon: IconItem) {
  const cached = editedSvgMap.value[icon.id]
  if (cached)
    return cached

  const state = editorStates.value[icon.id]
  const original = originalSvgMap.value[icon.id]
  if (state && original) {
    const { finalSvg } = buildEditedSvgPair(original, state, null)
    editedSvgMap.value[icon.id] = finalSvg
    return finalSvg
  }

  return null
}

function buildIconForSave(icon: IconItem, isEditorSave: boolean) {
  // 保存前确保拿到最新的编辑结果
  const editedSvg = getEditedSvg(icon)
  return {
    ...icon,
    name: isEditorSave ? buildCustomIconName(icon.name) : icon.name,
    svgContent: editedSvg || icon.svgContent,
  }
}

async function startSave(request: IconSaveRequest, isEditorSave = false) {
  if (isSaving.value)
    return

  // 逐图标保存，用于进度反馈与当前图标提示
  isSaving.value = true
  saveProgress.value = 0
  savingIconName.value = ''
  saveError.value = null
  needsConfirm.value = false
  pendingResponse.value = null
  saveSummary.value = null

  const items: IconSaveItem[] = []
  const total = request.icons.length

  try {
    for (let index = 0; index < request.icons.length; index++) {
      const icon = request.icons[index]
      const iconForSave = buildIconForSave(icon, isEditorSave)
      savingIconName.value = iconForSave.name

      const singleRequest: IconSaveRequest = {
        ...request,
        icons: [iconForSave],
      }

      const result = await saveIcons(singleRequest)
      if (result?.items?.length) {
        items.push(result.items[0])
      }
      else {
        items.push({
          id: iconForSave.id,
          name: iconForSave.name,
          success: false,
          savedPaths: [],
          error: '保存失败',
        })
      }

      saveProgress.value = Math.round(((index + 1) / total) * 100)
    }
  }
  catch (error) {
    console.error('保存图标失败:', error)
    saveError.value = String(error)
  }
  finally {
    isSaving.value = false
  }

  const successCount = items.filter(item => item.success).length
  const failedCount = items.length - successCount

  saveSummary.value = {
    items,
    successCount,
    failedCount,
    savePath: request.savePath,
  }

  pendingResponse.value = {
    saved_count: successCount,
    save_path: request.savePath,
    saved_names: items.filter(item => item.success).map(item => item.name),
    cancelled: false,
  }

  needsConfirm.value = true
}

async function handlePopupSave(request: IconSaveRequest) {
  if (!request.icons.length) {
    message.warning('没有可保存的图标')
    return
  }
  await startSave(request, false)
}

async function saveEditedIcon() {
  if (!activeIcon.value) {
    message.warning('请先选择要编辑的图标')
    return
  }

  if (!editorSavePath.value.trim()) {
    message.warning('请填写保存路径')
    return
  }

  const format = editorSaveFormat.value
  await startSave({
    icons: [activeIcon.value],
    savePath: editorSavePath.value,
    format,
    pngSize: needsPngSize.value ? editorPngSize.value : undefined,
  }, true)
}

async function handleConfirmClose() {
  if (!pendingResponse.value)
    return
  try {
    await invoke('send_mcp_response', { response: pendingResponse.value })
    await invoke('exit_app')
  }
  catch (error) {
    console.error('完成确认失败:', error)
    message.error('关闭失败，请重试')
  }
}

async function handleCancel() {
  try {
    const response = {
      saved_count: 0,
      save_path: '',
      saved_names: [],
      cancelled: true,
    }
    await invoke('send_mcp_response', { response })
    await invoke('exit_app')
  }
  catch (error) {
    console.error('Failed to cancel icon popup:', error)
    await invoke('exit_app')
  }
}
</script>

<template>
  <div class="h-screen flex flex-col bg-surface text-on-surface">
    <!-- 顶部导航栏 -->
    <div class="flex-shrink-0 h-14 border-b border-border flex items-center justify-between px-4 bg-surface-variant">
      <div class="flex items-center gap-2">
        <div class="i-carbon-image text-xl text-primary" />
        <span class="font-medium">图标工坊</span>
      </div>

      <div class="flex items-center gap-2">
        <n-button
          secondary
          size="small"
          :disabled="!selectedIcons.length || showProgressOverlay"
          @click="openEditorModal"
        >
          <template #icon>
            <div class="i-carbon-color-palette" />
          </template>
          SVG 编辑器
        </n-button>
        <n-button
          secondary
          type="error"
          size="small"
          :disabled="isSaving || needsConfirm"
          @click="handleCancel"
        >
          取消 / 关闭
        </n-button>
      </div>
    </div>

    <!-- 主内容区 -->
    <div class="flex-1 overflow-hidden p-4">
      <div class="relative h-full">
        <!-- 进度覆盖层 -->
        <transition
          enter-active-class="transition duration-200 ease-out"
          enter-from-class="opacity-0 translate-y-2"
          enter-to-class="opacity-100 translate-y-0"
          leave-active-class="transition duration-150 ease-in"
          leave-from-class="opacity-100 translate-y-0"
          leave-to-class="opacity-0 translate-y-2"
        >
          <div
            v-if="showProgressOverlay"
            class="absolute inset-0 z-30 flex items-center justify-center bg-surface backdrop-blur"
          >
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
                  <span>{{ saveProgress }}%</span>
                </div>
                <n-progress
                  type="line"
                  :percentage="saveProgress"
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

                <div v-if="saveSummary" class="grid grid-cols-2 gap-3 text-sm">
                  <div class="rounded-lg border border-border bg-surface p-3">
                    <div class="text-on-surface-secondary">
                      成功
                    </div>
                    <div class="text-lg font-semibold text-green-600">
                      {{ saveSummary.successCount }}
                    </div>
                  </div>
                  <div class="rounded-lg border border-border bg-surface p-3">
                    <div class="text-on-surface-secondary">
                      失败
                    </div>
                    <div class="text-lg font-semibold text-red-500">
                      {{ saveSummary.failedCount }}
                    </div>
                  </div>
                  <div class="col-span-2 rounded-lg border border-border bg-surface p-3">
                    <div class="text-on-surface-secondary">
                      保存路径
                    </div>
                    <div class="text-xs mt-1 break-all">
                      {{ saveSummary.savePath }}
                    </div>
                  </div>
                </div>

                <div v-if="saveError" class="text-xs text-red-500">
                  {{ saveError }}
                </div>

                <div class="flex justify-end">
                  <n-button type="primary" @click="handleConfirmClose">
                    确认并关闭
                  </n-button>
                </div>
              </div>
            </div>
          </div>
        </transition>

        <div class="h-full flex flex-col gap-4">
          <div class="flex-1 min-w-0 min-h-0 overflow-hidden icon-popup-scope">
            <IconWorkshop
              mode="popup"
              :active="true"
              :initial-query="props.initialQuery"
              :initial-style="props.initialStyle"
              :initial-save-path="props.initialSavePath"
              :project-root="props.projectRoot"
              :external-save="true"
              @save="handlePopupSave"
              @selection-change="handleSelectionChange"
              @icon-dblclick="handleIconDblClick"
              @icon-contextmenu="handleIconContextMenu"
            />
          </div>
        </div>
      </div>
    </div>

    <!-- 右键上下文菜单 -->
    <Teleport to="body">
      <transition
        enter-active-class="transition duration-100 ease-out"
        enter-from-class="opacity-0 scale-95"
        enter-to-class="opacity-100 scale-100"
        leave-active-class="transition duration-75 ease-in"
        leave-from-class="opacity-100 scale-100"
        leave-to-class="opacity-0 scale-95"
      >
        <div
          v-if="contextMenuVisible"
          class="fixed z-50 min-w-40 rounded-lg border border-border bg-surface-variant shadow-xl py-1"
          :style="{ left: `${contextMenuPosition.x}px`, top: `${contextMenuPosition.y}px` }"
          @click.stop
        >
          <div
            class="px-3 py-2 text-sm cursor-pointer hover:bg-surface-100 flex items-center gap-2"
            @click="contextMenuOpenEditor"
          >
            <div class="i-carbon-color-palette text-base" />
            <span>打开 SVG 编辑器</span>
          </div>
          <div
            class="px-3 py-2 text-sm cursor-pointer hover:bg-surface-100 flex items-center gap-2"
            @click="contextMenuCopySvg"
          >
            <div class="i-carbon-copy text-base" />
            <span>复制 SVG</span>
          </div>
        </div>
      </transition>
      <!-- 点击遮罩关闭菜单 -->
      <div
        v-if="contextMenuVisible"
        class="fixed inset-0 z-40"
        @click="closeContextMenu"
        @contextmenu.prevent="closeContextMenu"
      />
    </Teleport>

    <!-- SVG 编辑器弹窗 -->
    <div v-if="editorModalOpen" class="fixed inset-0 z-40 pointer-events-none">
      <div
        ref="editorModalRef"
        class="editor-floating pointer-events-auto"
        :class="[
          showProgressOverlay ? 'pointer-events-none opacity-60' : '',
          editorInteracting ? 'transition-none' : 'transition-all duration-150 ease-out',
        ]"
      >
        <div class="relative h-full rounded-2xl border border-slate-200/70 dark:border-white/10 bg-white/90 dark:bg-[#1f1f23] shadow-2xl flex flex-col overflow-hidden">
          <!-- 标题栏 -->
          <div class="flex-shrink-0 flex items-center justify-between px-4 py-3 border-b border-slate-200/70 dark:border-white/5 bg-slate-50/80 dark:bg-[#252529] cursor-move" @pointerdown.prevent="startDrag">
            <div class="flex items-center gap-2 select-none">
              <div class="i-carbon-color-palette text-lg text-slate-500" />
              <span class="font-semibold text-slate-700 dark:text-gray-200">SVG 编辑器</span>
              <n-tag :type="editorStatusTagType" size="small" round>
                {{ editorStatusLabel }}
              </n-tag>
            </div>
            <n-button size="small" quaternary circle @click="closeEditorModal">
              <template #icon><div class="i-carbon-close" /></template>
            </n-button>
          </div>

          <!-- 主内容区 -->
          <div class="flex-1 min-h-0 flex flex-col lg:flex-row gap-4 p-4">
            <!-- 左侧：选择 + 预览 -->
            <div class="flex flex-col gap-4 lg:basis-[46%] min-w-0">
              <div class="rounded-xl border border-slate-200/70 dark:border-white/10 bg-white/80 dark:bg-[#1a1a1d] p-3 space-y-2">
                <div class="flex items-center justify-between">
                  <label class="text-xs font-semibold text-slate-400 dark:text-gray-500 uppercase tracking-wider">当前图标</label>
                  <span class="text-xs text-slate-400">{{ selectedIcons.length }} 个已选</span>
                </div>
                <n-select
                  v-model:value="activeIconId"
                  size="small"
                  :options="selectedIcons.map(icon => ({ label: icon.name, value: icon.id }))"
                  placeholder="请选择图标"
                  :disabled="!selectedIcons.length"
                  virtual-scroll
                />
              </div>

              <div class="flex-1 min-h-[260px] lg:min-h-[360px] rounded-2xl border border-slate-200/70 dark:border-white/10 bg-slate-50/70 dark:bg-[#141417] p-4 flex flex-col gap-3">
                <div class="flex flex-wrap items-center justify-between gap-2">
                  <label class="text-xs font-semibold text-slate-400 dark:text-gray-500 uppercase tracking-wider">实时预览</label>
                  <div class="flex flex-wrap items-center gap-2">
                    <n-radio-group v-model:value="previewBackground" size="small">
                      <n-radio-button value="grid">网格</n-radio-button>
                      <n-radio-button value="light">浅色</n-radio-button>
                      <n-radio-button value="dark">深色</n-radio-button>
                    </n-radio-group>
                    <n-select
                      v-model:value="previewScale"
                      size="small"
                      :options="previewScaleOptions"
                      class="w-24"
                    />
                  </div>
                </div>

                <div class="relative flex-1 min-h-[200px] rounded-xl border border-slate-200/60 dark:border-white/10 overflow-hidden" :class="previewBackgroundClass">
                  <div v-if="previewBackground === 'grid'" class="absolute inset-0 pattern-grid opacity-15 pointer-events-none" />
                  <n-spin :show="previewBusy" size="small">
                    <div class="relative z-10 w-full h-full flex items-center justify-center px-3">
                      <n-skeleton v-if="editorStatus === 'loading'" text :repeat="3" class="w-full" />
                      <div v-else-if="editorStatus === 'error'" class="text-xs text-rose-400">
                        SVG 加载失败
                      </div>
                      <div v-else-if="editorPreviewSvg" class="w-full h-full flex items-center justify-center">
                        <div class="origin-center transition-transform duration-200" :class="previewScaleClass">
                          <div class="editor-preview w-full h-full" v-html="editorPreviewSvg" />
                        </div>
                      </div>
                      <div v-else class="text-xs text-slate-400">
                        请选择图标进行编辑
                      </div>
                    </div>
                  </n-spin>
                </div>

                <div class="flex items-center justify-between text-xs text-slate-400">
                  <span class="truncate">{{ activeIcon?.name || '未选择' }}</span>
                  <span v-if="activeState">尺寸 {{ activeState.width }} × {{ activeState.height }}</span>
                </div>
              </div>
            </div>

            <!-- 右侧：编辑面板 -->
            <div class="flex-1 min-w-0">
              <n-scrollbar class="h-full pr-2">
                <div v-if="editorStatus === 'loading'" class="space-y-4">
                  <n-skeleton text :repeat="2" />
                  <n-skeleton text :repeat="4" />
                  <n-skeleton text :repeat="4" />
                  <n-skeleton text :repeat="3" />
                </div>
                <div v-else class="space-y-4">
                  <n-collapse v-model:expanded-names="editorCollapseExpanded">
                    <n-collapse-item name="appearance" title="外观设置" :disabled="!activeState">
                      <div class="rounded-xl border border-slate-200/60 dark:border-white/10 bg-white/70 dark:bg-[#18181b] p-4 space-y-4">
                        <div class="flex items-center justify-between">
                          <span class="text-xs font-semibold text-slate-400 dark:text-gray-500 uppercase tracking-wider">全局颜色</span>
                          <n-switch
                            :value="activeState?.applyColor ?? false"
                            size="small"
                            :disabled="!activeState"
                            @update:value="(value: boolean) => updateActiveState('applyColor', value)"
                          />
                        </div>
                        <n-color-picker
                          :value="activeState?.color"
                          :swatches="mergedSwatches"
                          size="small"
                          :disabled="!activeState?.applyColor"
                          @update:value="(value: string) => value && updateActiveState('color', value)"
                        />
                      </div>
                    </n-collapse-item>

                    <n-collapse-item name="transform" title="尺寸与变换" :disabled="!activeState">
                      <div class="rounded-xl border border-slate-200/60 dark:border-white/10 bg-white/70 dark:bg-[#18181b] p-4 space-y-4">
                        <div class="grid grid-cols-2 gap-2">
                          <n-input-number
                            :value="activeState?.width"
                            size="small"
                            :min="8"
                            :max="512"
                            :disabled="!activeState"
                            @update:value="(value: number | null) => value !== null && updateActiveState('width', value)"
                          >
                            <template #prefix><span class="text-xs text-slate-400">W</span></template>
                          </n-input-number>
                          <n-input-number
                            :value="activeState?.height"
                            size="small"
                            :min="8"
                            :max="512"
                            :disabled="!activeState"
                            @update:value="(value: number | null) => value !== null && updateActiveState('height', value)"
                          >
                            <template #prefix><span class="text-xs text-slate-400">H</span></template>
                          </n-input-number>
                        </div>

                        <div class="flex flex-wrap gap-1.5">
                          <n-tag
                            v-for="size in sizePresets"
                            :key="size"
                            checkable
                            size="small"
                            class="cursor-pointer"
                            :checked="activeState?.width === size"
                            @click="applySizePreset(size)"
                          >
                            {{ size }}
                          </n-tag>
                        </div>

                        <n-input-number
                          :value="activeState?.rotate"
                          size="small"
                          :min="-180"
                          :max="180"
                          :disabled="!activeState"
                          @update:value="(value: number | null) => value !== null && updateActiveState('rotate', value)"
                        >
                          <template #prefix><span class="text-xs text-slate-400">旋转</span></template>
                          <template #suffix><span class="text-xs text-slate-400">°</span></template>
                        </n-input-number>

                        <div class="flex gap-2">
                          <n-button
                            class="flex-1"
                            size="small"
                            :type="activeState?.flipX ? 'primary' : 'default'"
                            :disabled="!activeState"
                            @click="toggleActiveState('flipX')"
                          >
                            <template #icon><div class="i-carbon-flip-horizontal" /></template>
                            水平
                          </n-button>
                          <n-button
                            class="flex-1"
                            size="small"
                            :type="activeState?.flipY ? 'primary' : 'default'"
                            :disabled="!activeState"
                            @click="toggleActiveState('flipY')"
                          >
                            <template #icon><div class="i-carbon-flip-vertical" /></template>
                            垂直
                          </n-button>
                        </div>
                      </div>
                    </n-collapse-item>

                    <n-collapse-item name="stroke" title="线条与圆角" :disabled="!activeState">
                      <div class="rounded-xl border border-slate-200/60 dark:border-white/10 bg-white/70 dark:bg-[#18181b] p-4 space-y-4">
                        <div class="flex items-center justify-between">
                          <span class="text-xs text-slate-500 dark:text-gray-400">圆角端点</span>
                          <n-switch
                            :value="activeState?.roundStroke ?? false"
                            size="small"
                            :disabled="!activeState"
                            @update:value="(value: boolean) => updateActiveState('roundStroke', value)"
                          />
                        </div>

                        <n-input-number
                          :value="activeState?.strokeWidth"
                          size="small"
                          :min="0"
                          :max="24"
                          :disabled="!activeState"
                          @update:value="(value: number | null) => updateActiveState('strokeWidth', value)"
                        >
                          <template #prefix><span class="text-xs text-slate-400">粗细</span></template>
                        </n-input-number>

                        <n-input-number
                          :value="activeState?.rectRadius"
                          size="small"
                          :min="0"
                          :max="32"
                          :disabled="!activeState"
                          @update:value="(value: number | null) => updateActiveState('rectRadius', value)"
                        >
                          <template #prefix><span class="text-xs text-slate-400">圆角</span></template>
                        </n-input-number>
                      </div>
                    </n-collapse-item>

                    <n-collapse-item name="element" title="元素级编辑">
                      <div class="rounded-xl border border-slate-200/60 dark:border-white/10 bg-white/70 dark:bg-[#18181b] p-4 space-y-4">
                        <n-input
                          v-model:value="elementSearch"
                          size="small"
                          clearable
                          placeholder="搜索元素（名称/类型）"
                          :disabled="!activeElementOptions.length"
                        >
                          <template #prefix><div class="i-carbon-search text-slate-400" /></template>
                        </n-input>

                        <n-select
                          v-model:value="activeElementKey"
                          size="small"
                          :options="filteredElementOptions.map(item => ({ label: item.label, value: item.key }))"
                          placeholder="选择线条元素"
                          :disabled="!activeElementOptions.length"
                          virtual-scroll
                        />

                        <div v-if="activeElementStyle" class="p-3 bg-white/80 dark:bg-[#121214] rounded-lg border border-slate-200/60 dark:border-white/10 space-y-3">
                          <div class="flex items-center justify-between">
                            <span class="text-xs font-medium text-slate-600 dark:text-gray-300">独立样式</span>
                            <n-switch
                              :value="activeElementStyle.enabled"
                              size="small"
                              @update:value="(value: boolean) => updateActiveElementStyle('enabled', value)"
                            />
                          </div>

                          <template v-if="activeElementStyle.enabled">
                            <n-color-picker
                              :value="activeElementStyle.strokeColor"
                              size="small"
                              :swatches="mergedSwatches"
                              @update:value="(value: string) => value && updateActiveElementStyle('strokeColor', value)"
                            />
                            <n-input-number
                              :value="activeElementStyle.strokeWidth"
                              size="small"
                              :min="0"
                              :step="0.5"
                              @update:value="(value: number | null) => updateActiveElementStyle('strokeWidth', value)"
                            >
                              <template #prefix><span class="text-xs">粗细</span></template>
                            </n-input-number>
                            <div class="flex items-center justify-between">
                              <span class="text-xs text-slate-500">圆角</span>
                              <n-switch
                                :value="activeElementStyle.roundStroke"
                                size="small"
                                @update:value="(value: boolean) => updateActiveElementStyle('roundStroke', value)"
                              />
                            </div>
                            <div class="flex justify-end">
                              <n-button size="tiny" secondary @click="resetActiveElementStyle">
                                重置当前元素
                              </n-button>
                            </div>
                          </template>
                        </div>

                        <div v-else-if="!activeElementOptions.length" class="text-xs text-slate-400 text-center py-2">
                          此图标无可编辑元素
                        </div>
                      </div>
                    </n-collapse-item>
                  </n-collapse>
                </div>
              </n-scrollbar>
            </div>
          </div>

          <!-- 底部操作栏 -->
          <div class="flex-shrink-0 p-4 border-t border-slate-200/70 dark:border-white/5 bg-slate-50/80 dark:bg-[#252529] space-y-3">
            <div class="flex flex-wrap items-center gap-3">
              <div class="flex items-center gap-2">
                <span class="text-xs text-slate-500">保存格式</span>
                <n-radio-group v-model:value="editorSaveFormat" size="small">
                  <n-radio-button value="svg">SVG</n-radio-button>
                  <n-radio-button value="png">PNG</n-radio-button>
                  <n-radio-button value="both">Both</n-radio-button>
                </n-radio-group>
              </div>
              <div v-if="needsPngSize" class="flex flex-wrap items-center gap-2">
                <span class="text-xs text-slate-500">PNG 尺寸</span>
                <n-input-number
                  :value="editorPngSize"
                  size="small"
                  :min="16"
                  :max="1024"
                  :step="2"
                  @update:value="updateEditorPngSize"
                />
                <div class="flex flex-wrap gap-1">
                  <n-tag
                    v-for="size in pngSizePresets"
                    :key="size"
                    checkable
                    size="small"
                    class="cursor-pointer"
                    :checked="editorPngSize === size"
                    @click="applyPngSizePreset(size)"
                  >
                    {{ size }}
                  </n-tag>
                </div>
              </div>
            </div>

            <div class="flex gap-2">
              <n-input
                v-model:value="editorSavePath"
                size="small"
                placeholder="保存目录"
                class="flex-1"
              >
                <template #prefix><div class="i-carbon-folder text-gray-400" /></template>
              </n-input>
              <n-button size="small" secondary @click="selectEditorDirectory">...</n-button>
            </div>
            
            <div class="grid grid-cols-3 gap-2">
              <n-button size="small" secondary :disabled="!editorPreviewSvg" @click="copyEditedSvg">
                <template #icon><div class="i-carbon-copy" /></template>
                复制
              </n-button>
              <n-button size="small" secondary :disabled="!activeState" @click="resetActiveEditor">
                复原
              </n-button>
              <n-button size="small" type="primary" :disabled="!activeIcon" @click="saveEditedIcon">
                保存
              </n-button>
            </div>
          </div>

          <!-- 缩放手柄 -->
          <div
            class="absolute bottom-1 right-1 w-4 h-4 cursor-nwse-resize z-20"
            @pointerdown.prevent="startResize"
          >
            <div class="absolute bottom-0 right-0 w-2 h-2 border-r-2 border-b-2 border-gray-300 dark:border-gray-600 rounded-br-sm" />
          </div>
        </div>
      </div>
    </div>
  </div>
</template>

<style scoped>
/* 弹窗编辑器基础布局 */
.editor-floating {
  position: absolute;
  left: 0;
  top: 0;
  will-change: transform, width, height;
}

/* 弹窗模式下放大图标预览与网格 - 大尺寸预览优化 */
.icon-popup-scope :deep(.icon-grid) {
  grid-template-columns: repeat(auto-fill, minmax(clamp(100px, 12vw, 140px), 1fr));
  gap: clamp(8px, 1.5vw, 12px);
}

.icon-popup-scope :deep(.icon-card) {
  padding: clamp(8px, 1.5vw, 12px);
  aspect-ratio: 1;
}

.icon-popup-scope :deep(.icon-preview) {
  width: clamp(48px, 8vw, 64px);
  height: clamp(48px, 8vw, 64px);
}

.icon-popup-scope :deep(.font-icon) {
  font-size: clamp(32px, 6vw, 48px);
}

.icon-popup-scope :deep(.skeleton-icon) {
  width: clamp(48px, 8vw, 64px);
  height: clamp(48px, 8vw, 64px);
}

.icon-popup-scope :deep(.icon-name) {
  font-size: clamp(10px, 1vw, 12px);
  margin-top: 4px;
}

/* 编辑器预览放大与选中高亮 */
.editor-preview :deep(svg) {
  width: 100%;
  height: 100%;
  max-width: 100%;
  max-height: 100%;
}

.editor-preview :deep([data-editor-focus='true']) {
  filter: drop-shadow(0 0 6px rgba(126, 156, 180, 0.6));
}

/* 网格背景图案 */
.pattern-grid {
  background-image:
    linear-gradient(to right, rgba(255, 255, 255, 0.03) 1px, transparent 1px),
    linear-gradient(to bottom, rgba(255, 255, 255, 0.03) 1px, transparent 1px);
  background-size: 16px 16px;
}

/* 自定义滚动条 */
.custom-scrollbar::-webkit-scrollbar {
  width: 6px;
}
.custom-scrollbar::-webkit-scrollbar-track {
  background: transparent;
}
.custom-scrollbar::-webkit-scrollbar-thumb {
  background-color: rgba(156, 163, 175, 0.2);
  border-radius: 3px;
}
.custom-scrollbar::-webkit-scrollbar-thumb:hover {
  background-color: rgba(156, 163, 175, 0.4);
}
</style>
