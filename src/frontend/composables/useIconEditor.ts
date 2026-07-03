// 图标编辑器 Hook
// 管理 SVG 编辑状态与变换逻辑：全局颜色、尺寸、旋转、翻转、描边、圆角与逐元素样式覆盖
// 从 IconPopupMode.vue（原上帝组件）拆分而来，供编辑抽屉与右键菜单复用

import type { Ref } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import { computed, ref, watch } from 'vue'
import type { IconItem } from '../types/icon'

/** 可编辑的 SVG 元素选项 */
export interface SvgElementOption {
  key: string
  label: string
  tag: string
}

/** 单个 SVG 元素的独立样式覆盖 */
export interface SvgElementStyle {
  enabled: boolean
  strokeColor: string
  strokeWidth: number | null
  roundStroke: boolean
}

/** 单个图标的编辑状态 */
export interface IconEditorState {
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

/** 预设色板（莫兰迪色系） */
const PALETTE_COLORS = [
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

/**
 * 图标编辑器 Hook
 *
 * @param selectedIcons 当前选中的图标列表（响应式），编辑器跟随选中项自动切换
 */
export function useIconEditor(selectedIcons: Ref<IconItem[]>) {
  // ============ 状态定义 ============

  /** 当前活动图标 ID */
  const activeIconId = ref<number | null>(null)

  /** 编辑器加载状态 */
  const editorStatus = ref<'idle' | 'loading' | 'ready' | 'error'>('idle')

  /** 编辑后的预览 SVG（带选中元素高亮标记） */
  const editorPreviewSvg = ref<string>('')

  /** 预览是否正在防抖更新 */
  const previewUpdating = ref(false)

  /** 最近使用的颜色 */
  const recentColors = ref<string[]>([])

  /** 元素搜索关键词 */
  const elementSearch = ref('')

  /** 每个图标的编辑状态缓存 */
  const editorStates = ref<Record<number, IconEditorState>>({})
  /** 每个图标的原始 SVG 缓存 */
  const originalSvgMap = ref<Record<number, string>>({})
  /** 每个图标编辑后的最终 SVG 缓存 */
  const editedSvgMap = ref<Record<number, string>>({})
  /** 每个图标的可编辑元素列表缓存 */
  const elementOptionsMap = ref<Record<number, SvgElementOption[]>>({})
  /** 每个图标元素的默认样式缓存 */
  const elementDefaultStyles = ref<Record<number, Record<string, SvgElementStyle>>>({})

  /** 当前活动图标的编辑状态 */
  const activeState = ref<IconEditorState | null>(null)

  // ============ 计算属性 ============

  /** 当前活动图标 */
  const activeIcon = computed(() => {
    if (!selectedIcons.value.length)
      return null
    return selectedIcons.value.find(icon => icon.id === activeIconId.value) || selectedIcons.value[0] || null
  })

  /** 当前图标的可编辑元素列表 */
  const activeElementOptions = computed(() => {
    const icon = activeIcon.value
    if (!icon)
      return []
    return elementOptionsMap.value[icon.id] || []
  })

  /** 当前选中的元素 key（读写代理到 activeState） */
  const activeElementKey = computed({
    get: () => activeState.value?.activeElementKey ?? null,
    set: (value) => {
      if (activeState.value)
        activeState.value.activeElementKey = value
    },
  })

  /** 当前选中元素的独立样式 */
  const activeElementStyle = computed(() => {
    const state = activeState.value
    if (!state || !state.activeElementKey)
      return null
    return state.elementStyles[state.activeElementKey] || null
  })

  /** 按关键词过滤后的元素列表（当前选中项始终置顶保留） */
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

  /** 最近颜色 + 预设色板合并去重后的色板 */
  const mergedSwatches = computed(() => {
    const list = [...recentColors.value, ...PALETTE_COLORS]
    const seen = new Set<string>()
    return list.filter((color) => {
      const key = color.toUpperCase()
      if (seen.has(key))
        return false
      seen.add(key)
      return true
    }).slice(0, 12)
  })

  // ============ 监听器 ============

  // 选中列表变化时维护活动图标
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

  // 活动图标变化时准备编辑器
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

  // 编辑状态任意变化时防抖刷新预览
  watch(activeState, () => {
    schedulePreviewUpdate()
  }, { deep: true })

  // 切换元素时确保存在样式对象
  watch(activeElementKey, () => {
    ensureActiveElementStyle()
  }, { immediate: true })

  // 应用全局颜色时记录最近颜色
  watch(() => [activeState.value?.color, activeState.value?.applyColor], ([color, apply]) => {
    if (apply && color && typeof color === 'string')
      pushRecentColor(color)
  })

  // ============ 预览更新 ============

  let previewTimer: number | null = null

  /** 防抖更新预览（200ms） */
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

  // ============ 编辑器准备 ============

  /** 加载图标 SVG 并初始化编辑状态 */
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

  /** 获取图标原始 SVG（优先缓存，其次自带内容，最后请求后端兜底） */
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

  /** 解析并缓存图标的可编辑元素 */
  function ensureEditableElements(icon: IconItem, svg: string) {
    if (elementOptionsMap.value[icon.id])
      return
    const { options, defaults } = extractEditableElements(svg)
    elementOptionsMap.value[icon.id] = options
    elementDefaultStyles.value[icon.id] = defaults
  }

  /** 从 SVG 中提取可编辑的线条元素与其默认样式 */
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

  /** 收集 SVG 内可编辑节点（排除 defs 内的定义节点） */
  function collectEditableNodes(svgEl: SVGSVGElement) {
    const nodes = Array.from(svgEl.querySelectorAll('path, line, rect, circle, ellipse, polyline, polygon'))
    return nodes.filter(node => !node.closest('defs'))
  }

  /** 生成元素的稳定 key */
  function buildElementKey(node: Element, index: number) {
    return `${node.tagName.toLowerCase()}-${index + 1}`
  }

  /** 从节点属性推导默认元素样式 */
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

  /** 确保当前选中元素存在样式对象（懒初始化） */
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

  // ============ 状态更新方法 ============

  /** 更新当前编辑状态的单个字段 */
  function updateActiveState<K extends keyof IconEditorState>(key: K, value: IconEditorState[K]) {
    const state = activeState.value
    if (!state)
      return
    state[key] = value
  }

  /** 切换翻转开关 */
  function toggleActiveState(key: 'flipX' | 'flipY') {
    const state = activeState.value
    if (!state)
      return
    state[key] = !state[key]
  }

  /** 应用尺寸预设（宽高同值） */
  function applySizePreset(size: number) {
    updateActiveState('width', size)
    updateActiveState('height', size)
  }

  /** 更新当前元素的独立样式字段 */
  function updateActiveElementStyle<K extends keyof SvgElementStyle>(key: K, value: SvgElementStyle[K]) {
    const style = activeElementStyle.value
    if (!style)
      return
    style[key] = value
    schedulePreviewUpdate()
  }

  /** 重置当前元素样式为解析出的默认值 */
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

  /** 复原当前图标的所有编辑 */
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

  /** 记录最近使用颜色（去重，最多 6 个） */
  function pushRecentColor(color: string) {
    const normalized = color.toUpperCase()
    const list = recentColors.value.filter(item => item.toUpperCase() !== normalized)
    list.unshift(color)
    recentColors.value = list.slice(0, 6)
  }

  // ============ SVG 构建 ============

  /** 创建默认编辑状态 */
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

  /** 解析 SVG 尺寸（width/height 属性优先，其次 viewBox，兜底 64） */
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

  /** 按编辑状态构建最终 SVG 与预览 SVG（预览带选中元素高亮标记） */
  function buildEditedSvgPair(svg: string, state: IconEditorState, focusKey: string | null) {
    try {
      const doc = new DOMParser().parseFromString(svg, 'image/svg+xml')
      const svgEl = doc.querySelector('svg')
      if (!svgEl)
        return { finalSvg: svg, previewSvg: svg }

      // 按当前编辑状态应用颜色、尺寸与变换
      const viewBoxInfo = readViewBox(svgEl, state)

      // 移除内联样式，避免 width/height 被固定为 1em 导致预览过小
      svgEl.removeAttribute('style')

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

  /** 读取 viewBox 信息（缺失时以编辑状态尺寸兜底） */
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

  // ============ 保存辅助 ============

  /** 生成带时间戳的自定义图标文件名，避免覆盖原图标 */
  function buildCustomIconName(name: string) {
    const now = new Date()
    const timestamp = `${now.getFullYear()}${String(now.getMonth() + 1).padStart(2, '0')}${String(now.getDate()).padStart(2, '0')}`
      + `${String(now.getHours()).padStart(2, '0')}${String(now.getMinutes()).padStart(2, '0')}${String(now.getSeconds()).padStart(2, '0')}`
    const random = Math.random().toString(36).slice(2, 6)
    return `${name}-${timestamp}-${random}`
  }

  /** 获取图标编辑后的 SVG（优先缓存，否则按状态即时构建） */
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

  /** 构建用于保存的图标对象（编辑保存时附加时间戳文件名） */
  function buildIconForSave(icon: IconItem, isEditorSave: boolean): IconItem {
    // 保存前确保拿到最新的编辑结果
    const editedSvg = getEditedSvg(icon)
    return {
      ...icon,
      name: isEditorSave ? buildCustomIconName(icon.name) : icon.name,
      svgContent: editedSvg || icon.svgContent,
    }
  }

  // ============ 返回值 ============

  return {
    // 状态
    activeIconId,
    activeIcon,
    activeState,
    editorStatus,
    editorPreviewSvg,
    previewUpdating,
    recentColors,
    mergedSwatches,
    elementSearch,
    activeElementOptions,
    filteredElementOptions,
    activeElementKey,
    activeElementStyle,

    // 方法
    updateActiveState,
    toggleActiveState,
    applySizePreset,
    updateActiveElementStyle,
    resetActiveElementStyle,
    resetActiveEditor,
    getEditedSvg,
    buildIconForSave,
  }
}

/** useIconEditor 的返回类型（供组件 props 类型标注） */
export type IconEditor = ReturnType<typeof useIconEditor>
