<script setup lang="ts">
import type { IconItem, IconSaveItem, IconSaveRequest, IconSaveResult } from '../../../types/icon'
import { invoke } from '@tauri-apps/api/core'
import { useMessage } from 'naive-ui'
import { computed, nextTick, onBeforeUnmount, onMounted, ref, watch } from 'vue'
import { useIconSearch } from '../../../composables/useIconSearch'
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
  if (apply && color)
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

function schedulePreviewUpdate() {
  if (previewTimer)
    window.clearTimeout(previewTimer)

  previewTimer = window.setTimeout(() => {
    const icon = activeIcon.value
    const state = activeState.value
    if (!icon || !state)
      return

    const originalSvg = originalSvgMap.value[icon.id]
    if (!originalSvg)
      return

    const { finalSvg, previewSvg } = buildEditedSvgPair(originalSvg, state, state.activeElementKey)
    editorPreviewSvg.value = previewSvg
    editedSvgMap.value[icon.id] = finalSvg
  }, 200)
}

async function prepareEditor(icon: IconItem) {
  editorStatus.value = 'loading'

  const originalSvg = await ensureOriginalSvg(icon)
  if (!originalSvg) {
    editorStatus.value = 'error'
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
    if (focusNode) {
      focusNode.setAttribute('data-editor-focus', 'true')
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

  await startSave({
    icons: [activeIcon.value],
    savePath: editorSavePath.value,
    format: 'svg',
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
          <div class="flex-1 min-w-0 icon-popup-scope">
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
            />
          </div>
        </div>
      </div>
    </div>

    <!-- SVG 编辑器弹窗 -->
    <div v-if="editorModalOpen" class="fixed inset-0 z-40 pointer-events-none">
      <div
        ref="editorModalRef"
        class="editor-floating pointer-events-auto"
        :class="showProgressOverlay ? 'pointer-events-none opacity-60' : ''"
      >
        <div class="relative h-full rounded-2xl border border-border bg-surface-variant shadow-xl flex flex-col overflow-hidden">
          <div class="flex items-center justify-between px-4 py-3 border-b border-border bg-surface">
            <div class="flex items-center gap-2 cursor-move select-none" @pointerdown.prevent="startDrag">
              <div class="i-carbon-draggable text-lg text-on-surface-secondary" />
              <span class="font-medium">SVG 编辑器</span>
            </div>
            <n-button size="small" tertiary @click="closeEditorModal">
              关闭
            </n-button>
          </div>

          <div class="flex-1 overflow-y-auto p-4 space-y-4">
            <div class="space-y-2">
              <div class="text-xs text-on-surface-secondary">
                当前选中
              </div>
              <n-select
                v-model:value="activeIconId"
                size="small"
                :options="selectedIcons.map(icon => ({ label: icon.name, value: icon.id }))"
                placeholder="请选择图标"
                :disabled="!selectedIcons.length"
              />
            </div>

            <div class="rounded-xl border border-border bg-surface p-4">
              <div class="text-xs text-on-surface-secondary mb-2">
                实时预览
              </div>
              <div class="editor-preview aspect-square w-full max-w-64 rounded-xl bg-surface-100 flex items-center justify-center mx-auto overflow-hidden">
                <n-skeleton v-if="editorStatus === 'loading'" text :repeat="4" class="w-full" />
                <div v-else-if="editorStatus === 'error'" class="text-xs text-red-500">
                  SVG 加载失败
                </div>
                <div v-else-if="editorPreviewSvg" class="w-full h-full" v-html="editorPreviewSvg" />
                <div v-else class="text-xs text-on-surface-secondary">
                  请选择图标进行编辑
                </div>
              </div>
            </div>

            <div class="space-y-2">
              <div class="text-xs text-on-surface-secondary">
                线条元素
              </div>
              <n-select
                v-model:value="activeElementKey"
                size="small"
                :options="activeElementOptions.map(item => ({ label: item.label, value: item.key }))"
                placeholder="选择线条元素"
                :disabled="!activeElementOptions.length"
              />
              <div v-if="!activeElementOptions.length" class="text-xs text-on-surface-secondary">
                暂无可编辑线条
              </div>
            </div>

            <div class="space-y-3">
              <div class="flex items-center justify-between">
                <span class="text-sm font-medium">线条样式</span>
                <n-switch
                  :value="activeElementStyle?.enabled ?? false"
                  size="small"
                  :disabled="!activeElementStyle"
                  @update:value="value => updateActiveElementStyle('enabled', value)"
                />
              </div>
              <div class="text-xs text-on-surface-secondary">
                启用后仅作用于当前线条
              </div>
              <n-color-picker
                :value="activeElementStyle?.strokeColor ?? '#8B8B8B'"
                :swatches="paletteColors"
                size="small"
                :disabled="!activeElementStyle?.enabled"
                @update:value="value => value && updateActiveElementStyle('strokeColor', value)"
              />
              <n-input-number
                :value="activeElementStyle?.strokeWidth ?? null"
                size="small"
                :min="0"
                :max="24"
                :disabled="!activeElementStyle?.enabled"
                @update:value="value => updateActiveElementStyle('strokeWidth', value)"
              >
                <template #prefix>
                  线条粗细
                </template>
              </n-input-number>
              <div class="flex items-center justify-between">
                <span class="text-xs text-on-surface-secondary">线条圆角</span>
                <n-switch
                  :value="activeElementStyle?.roundStroke ?? false"
                  size="small"
                  :disabled="!activeElementStyle?.enabled"
                  @update:value="value => updateActiveElementStyle('roundStroke', value)"
                />
              </div>
              <div class="flex justify-end">
                <n-button size="tiny" quaternary :disabled="!activeElementStyle" @click="resetActiveElementStyle">
                  重置线条
                </n-button>
              </div>
            </div>

            <div class="space-y-3">
              <div class="flex items-center justify-between">
                <span class="text-sm font-medium">颜色</span>
                <n-switch
                  :value="activeState?.applyColor ?? false"
                  size="small"
                  :disabled="!activeState"
                  @update:value="value => updateActiveState('applyColor', value)"
                />
              </div>
              <n-color-picker
                :value="activeState?.color"
                :swatches="paletteColors"
                size="small"
                :disabled="!activeState"
                @update:value="value => value && updateActiveState('color', value)"
              />
              <div class="text-xs text-on-surface-secondary">
                最近颜色
              </div>
              <div class="flex flex-wrap gap-2">
                <n-button
                  v-for="color in recentColors"
                  :key="color"
                  size="tiny"
                  quaternary
                  :disabled="!activeState"
                  @click="updateActiveState('color', color)"
                >
                  {{ color }}
                </n-button>
                <div v-if="!recentColors.length" class="text-xs text-on-surface-secondary">
                  暂无
                </div>
              </div>
            </div>

            <div class="space-y-3">
              <div class="text-sm font-medium">
                尺寸
              </div>
              <div class="grid grid-cols-2 gap-2">
                <n-input-number
                  :value="activeState?.width"
                  size="small"
                  :min="8"
                  :max="512"
                  :disabled="!activeState"
                  @update:value="value => value !== null && updateActiveState('width', value)"
                >
                  <template #prefix>
                    W
                  </template>
                </n-input-number>
                <n-input-number
                  :value="activeState?.height"
                  size="small"
                  :min="8"
                  :max="512"
                  :disabled="!activeState"
                  @update:value="value => value !== null && updateActiveState('height', value)"
                >
                  <template #prefix>
                    H
                  </template>
                </n-input-number>
              </div>
              <div class="flex flex-wrap gap-2">
                <n-button
                  v-for="size in sizePresets"
                  :key="size"
                  size="tiny"
                  quaternary
                  :disabled="!activeState"
                  @click="applySizePreset(size)"
                >
                  {{ size }}
                </n-button>
              </div>
            </div>

            <div class="space-y-3">
              <div class="text-sm font-medium">
                变换
              </div>
              <n-input-number
                :value="activeState?.rotate"
                size="small"
                :min="-180"
                :max="180"
                :disabled="!activeState"
                @update:value="value => value !== null && updateActiveState('rotate', value)"
              >
                <template #prefix>
                  旋转
                </template>
              </n-input-number>
              <div class="flex gap-2">
                <n-button
                  size="small"
                  :type="activeState?.flipX ? 'primary' : 'default'"
                  :disabled="!activeState"
                  @click="toggleActiveState('flipX')"
                >
                  <template #icon>
                    <div class="i-carbon-flip-horizontal" />
                  </template>
                  水平翻转
                </n-button>
                <n-button
                  size="small"
                  :type="activeState?.flipY ? 'primary' : 'default'"
                  :disabled="!activeState"
                  @click="toggleActiveState('flipY')"
                >
                  <template #icon>
                    <div class="i-carbon-flip-vertical" />
                  </template>
                  垂直翻转
                </n-button>
              </div>
            </div>

            <div class="space-y-3">
              <div class="text-sm font-medium">
                形状微调
              </div>
              <div class="flex items-center justify-between">
                <span class="text-xs text-on-surface-secondary">线条圆角</span>
                <n-switch
                  :value="activeState?.roundStroke ?? false"
                  size="small"
                  :disabled="!activeState"
                  @update:value="value => updateActiveState('roundStroke', value)"
                />
              </div>
              <n-input-number
                :value="activeState?.strokeWidth"
                size="small"
                :min="0"
                :max="24"
                :disabled="!activeState"
                @update:value="value => updateActiveState('strokeWidth', value)"
              >
                <template #prefix>
                  线条粗细
                </template>
              </n-input-number>
              <n-input-number
                :value="activeState?.rectRadius"
                size="small"
                :min="0"
                :max="32"
                :disabled="!activeState"
                @update:value="value => updateActiveState('rectRadius', value)"
              >
                <template #prefix>
                  矩形圆角
                </template>
              </n-input-number>
            </div>

            <div class="space-y-3">
              <div class="text-sm font-medium">
                编辑器保存
              </div>
              <div class="flex gap-2">
                <n-input
                  v-model:value="editorSavePath"
                  size="small"
                  placeholder="保存目录"
                >
                  <template #prefix>
                    <div class="i-carbon-folder" />
                  </template>
                </n-input>
                <n-button size="small" secondary @click="selectEditorDirectory">
                  选择
                </n-button>
              </div>
              <div class="flex flex-wrap gap-2">
                <n-button size="small" quaternary :disabled="!editorPreviewSvg" @click="copyEditedSvg">
                  复制 SVG
                </n-button>
                <n-button size="small" quaternary :disabled="!activeState" @click="resetActiveEditor">
                  重置编辑
                </n-button>
                <n-button type="primary" size="small" :disabled="!activeIcon" @click="saveEditedIcon">
                  保存此 SVG
                </n-button>
              </div>
            </div>
          </div>

          <div
            class="absolute bottom-2 right-2 h-5 w-5 cursor-se-resize text-on-surface-secondary/70"
            @pointerdown.prevent="startResize"
          >
            <div class="h-full w-full border-r border-b border-on-surface-secondary/40 rotate-45" />
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

/* 弹窗模式下放大图标预览与网格 */
.icon-popup-scope :deep(.icon-grid) {
  grid-template-columns: repeat(auto-fill, minmax(clamp(120px, 18vw, 180px), 1fr));
  gap: clamp(10px, 1.6vw, 16px);
}

.icon-popup-scope :deep(.icon-card) {
  padding: clamp(12px, 1.8vw, 18px);
}

.icon-popup-scope :deep(.icon-preview) {
  width: clamp(52px, 7vw, 110px);
  height: clamp(52px, 7vw, 110px);
}

.icon-popup-scope :deep(.font-icon) {
  font-size: clamp(36px, 6vw, 96px);
}

.icon-popup-scope :deep(.skeleton-icon) {
  width: clamp(52px, 7vw, 110px);
  height: clamp(52px, 7vw, 110px);
}

/* 编辑器预览放大与选中高亮 */
.editor-preview :deep(svg) {
  width: 100%;
  height: 100%;
}

.editor-preview :deep([data-editor-focus='true']) {
  filter: drop-shadow(0 0 6px rgba(126, 156, 180, 0.6));
}
</style>
