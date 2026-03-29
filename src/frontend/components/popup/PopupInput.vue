<script setup lang="ts">
import { invoke } from '@tauri-apps/api/core'
import { listen } from '@tauri-apps/api/event'
import { getCurrentWebviewWindow } from '@tauri-apps/api/webviewWindow'
import { getCurrentWindow } from '@tauri-apps/api/window'
import StarterKit from '@tiptap/starter-kit'
import { EditorContent, useEditor } from '@tiptap/vue-3'
import { useStorage } from '@vueuse/core'
import { useSortable } from '@vueuse/integrations/useSortable'
import { useMessage } from 'naive-ui'
import { computed, nextTick, onMounted, onUnmounted, ref, shallowRef, watch } from 'vue'
import { useKeyboard } from '../../composables/useKeyboard'
import { useMcpToolsReactive } from '../../composables/useMcpTools'
import type { CustomPrompt, FileReferenceAttachment, McpRequest } from '../../types/popup'
import { buildConditionalContext } from '../../utils/conditionalContext'
import { compressImage, compressionSummary } from '../../utils/imageCompression'
import AppModal from '../common/AppModal.vue'
import type { InlineBadgeAttrs } from './extensions/InlineBadge'
import { InlineBadge } from './extensions/InlineBadge'

// TipTap badge 操作辅助
function editorInsertBadge(ed: ReturnType<typeof useEditor>['value'], attrs: InlineBadgeAttrs) {
  if (!ed) return
  ed.chain().focus().insertContent([
    { type: 'inlineBadge', attrs },
    { type: 'text', text: ' ' },
  ]).run()
}

function editorRemoveBadge(ed: ReturnType<typeof useEditor>['value'], key: 'identity' | 'imageBadgeId', value: string, all = false) {
  if (!ed) return
  ed.chain().command(({ tr, state }) => {
    const targets: { from: number, to: number }[] = []
    state.doc.descendants((node, pos) => {
      if (node.type.name === 'inlineBadge' && node.attrs[key] === value) {
        const end = pos + node.nodeSize
        const after = state.doc.nodeAt(end)
        const deleteEnd = (after?.isText && /^[\s\u00a0]/.test(after.text || '')) ? end + 1 : end
        targets.push({ from: pos, to: deleteEnd })
        if (!all) return false
      }
    })
    for (let i = targets.length - 1; i >= 0; i--) {
      tr.delete(targets[i].from, targets[i].to)
    }
    return targets.length > 0
  }).run()
}

interface Props {
  request: McpRequest | null
  loading?: boolean
  submitting?: boolean
  enhanceEnabled?: boolean
}

interface Emits {
  update: [data: {
    userInput: string
    rawUserInput: string
    conditionalContext: string
    selectedOptions: string[]
    draggedImages: string[]
    imageNames: string[]
    referencedFiles: FileReferenceAttachment[]
  }]
  imageAdd: [image: string]
  imageRemove: [index: number]
  enhance: []
  openMcpToolsTab: []
}

const props = withDefaults(defineProps<Props>(), {
  loading: false,
  submitting: false,
  enhanceEnabled: false,
})

const emit = defineEmits<Emits>()

// 响应式数据
const userInput = ref('')
const selectedOptions = ref<string[]>([])
const uploadedImages = ref<string[]>([])
const imageNames = ref<string[]>([])
const referencedFiles = ref<FileReferenceAttachment[]>([])
const isDragOver = ref(false)
// 自定义prompt相关状态
const customPrompts = ref<CustomPrompt[]>([])
const customPromptEnabled = ref(true)
const showInsertDialog = ref(false)
const pendingPromptContent = ref('')

// 移除条件性prompt状态管理，直接使用prompt的current_state

// 分离普通prompt和条件性prompt
const normalPrompts = computed(() =>
  customPrompts.value.filter(prompt => prompt.type === 'normal' || !prompt.type),
)

const conditionalPrompts = computed(() =>
  customPrompts.value.filter(prompt => prompt.type === 'conditional'),
)

// MCP 工具状态管理
const { mcpTools, loadMcpTools } = useMcpToolsReactive()

// 检查关联的 MCP 工具是否启用
function isMcpToolEnabled(toolId?: string): boolean {
  if (!toolId) {
    return true // 没有关联工具时默认可用
  }
  const tool = mcpTools.value.find(t => t.id === toolId)
  return tool?.enabled ?? false
}

// 获取 MCP 工具名称（用于提示文案）
function getMcpToolName(toolId?: string): string {
  if (!toolId) {
    return ''
  }
  const tool = mcpTools.value.find(t => t.id === toolId)
  return tool?.name ?? toolId
}

// 拖拽排序相关状态
const promptContainer = ref<HTMLElement | null>(null)
const sortablePrompts = shallowRef<CustomPrompt[]>([])
const { start, stop } = useSortable(promptContainer, sortablePrompts, {
  animation: 200,
  ghostClass: 'sortable-ghost',
  chosenClass: 'sortable-chosen',
  dragClass: 'sortable-drag',
  handle: '.drag-handle',
  forceFallback: true,
  fallbackTolerance: 3,
  onEnd: (evt) => {
    if (evt.oldIndex !== evt.newIndex && evt.oldIndex !== undefined && evt.newIndex !== undefined) {
      const newList = [...sortablePrompts.value]
      const [movedItem] = newList.splice(evt.oldIndex, 1)
      newList.splice(evt.newIndex, 0, movedItem)
      sortablePrompts.value = newList

      const conditionalPromptsList = customPrompts.value.filter(prompt => prompt.type === 'conditional')
      customPrompts.value = [...sortablePrompts.value, ...conditionalPromptsList]
      savePromptOrder()
    }
  },
  onMove: () => true,
})

// 使用键盘快捷键 composable
const { pasteShortcut } = useKeyboard()

const message = useMessage()
const imageCompressionEnabled = ref(true)

// 计算属性
const hasOptions = computed(() => (props.request?.predefined_options?.length ?? 0) > 0)
const canSubmit = computed(() => {
  const hasOptionsSelected = selectedOptions.value.length > 0
  const hasInputText = userInput.value.trim().length > 0
  const hasImages = uploadedImages.value.length > 0
  const hasFiles = referencedFiles.value.length > 0

  if (hasOptions.value) {
    return hasOptionsSelected || hasInputText || hasImages || hasFiles
  }
  return hasInputText || hasImages || hasFiles
})
const canEnhance = computed(() => userInput.value.trim().length > 0)

// 工具栏状态文本
const statusText = computed(() => {
  const hasInput = selectedOptions.value.length > 0
    || uploadedImages.value.length > 0
    || referencedFiles.value.length > 0
    || userInput.value.trim().length > 0

  // 如果有任何输入内容，返回空字符串让 PopupActions 显示快捷键
  if (hasInput) {
    return ''
  }

  return '等待输入...'
})

// 悬浮/固定相关状态
const isFloating = useStorage('popup-input-floating', false)
const isFloatingExpanded = ref(false)

function toggleFloatingExpanded() {
  isFloatingExpanded.value = !isFloatingExpanded.value
}

function toggleFloating() {
  isFloating.value = !isFloating.value
}

// 发送更新事件
function emitUpdate() {
  const conditionalContent = generateConditionalContent()
  const finalUserInput = userInput.value + conditionalContent

  emit('update', {
    userInput: finalUserInput,
    rawUserInput: userInput.value,
    conditionalContext: conditionalContent,
    selectedOptions: [...selectedOptions.value],
    draggedImages: [...uploadedImages.value],
    imageNames: [...imageNames.value],
    referencedFiles: [...referencedFiles.value],
  })
}

// 处理选项变化
function handleOptionChange(option: string, checked: boolean) {
  if (checked) {
    selectedOptions.value.push(option)
  }
  else {
    const idx = selectedOptions.value.indexOf(option)
    if (idx > -1)
      selectedOptions.value.splice(idx, 1)
  }
  emitUpdate()
}

// 处理选项切换（整行点击）
function handleOptionToggle(option: string) {
  const idx = selectedOptions.value.indexOf(option)
  if (idx > -1) {
    selectedOptions.value.splice(idx, 1)
  }
  else {
    selectedOptions.value.push(option)
  }
  emitUpdate()
}

// ============ TipTap 编辑器辅助 ============

const URL_PATTERN = /^https?:\/\/\S+$/i
const PATH_PATTERN = /^(?:[a-zA-Z]:[\\\/]|[\/]).+/

function getReferenceIdentity(ref: FileReferenceAttachment): string {
  return ref.type === 'url'
    ? (ref.url || '').trim().toLowerCase()
    : (ref.path || '').trim().toLowerCase()
}

function getSerializedReferenceText(ref: FileReferenceAttachment): string {
  const displayName = getReferenceDisplayLabel(ref)
  const typeTag = ref.type === 'url' ? 'url' : (ref.kind === 'directory' ? 'dir' : 'file')
  return `[${typeTag}: ${displayName}]`
}

function getReferenceKindLabel(ref: FileReferenceAttachment): string {
  if (ref.type === 'url') return 'URL'
  if (ref.kind === 'directory') return '目录'
  return '文件'
}

function getReferenceDisplayLabel(ref: FileReferenceAttachment): string {
  if (ref.type === 'url') {
    const url = ref.url || ''
    return url.replace(/^https?:\/\//, '').replace(/\/+$/, '')
  }
  const filePath = ref.path || ''
  return filePath.split(/[/\\]/).pop() || filePath
}

function addFileReference(file: FileReferenceAttachment): boolean {
  const identity = getReferenceIdentity(file)
  if (!referencedFiles.value.some(item => getReferenceIdentity(item) === identity)) {
    referencedFiles.value.push(file)
  }
  return true
}

// ---- TipTap JSON 序列化 ----

function serializeNode(node: any): string {
  if (node.type === 'text') return node.text || ''
  if (node.type === 'inlineBadge') return node.attrs?.serialized || ''
  if (node.type === 'hardBreak') return '\n'
  if (node.type === 'paragraph') {
    const inner = (node.content || []).map(serializeNode).join('')
    return `${inner}\n`
  }
  return (node.content || []).map(serializeNode).join('')
}

function serializeEditorContent(): string {
  if (!editor.value) return ''
  const json = editor.value.getJSON()
  return serializeNode(json).replace(/\n$/, '')
}

function walkNodes(node: any, callback: (node: any) => void) {
  callback(node)
  if (node.content) {
    for (const child of node.content) {
      walkNodes(child, callback)
    }
  }
}

// ---- 数据同步 ----

function syncDataWithEditor() {
  if (!editor.value) return
  const json = editor.value.getJSON()
  const presentRefIds = new Set<string>()
  const presentImgIds = new Set<string>()

  walkNodes(json, (node: any) => {
    if (node.type === 'inlineBadge') {
      const attrs = node.attrs
      if (attrs?.badgeType === 'image' && attrs?.imageBadgeId) {
        const imgId = attrs.imageBadgeId
        presentImgIds.add(imgId)
        if (!imageBadgeMap.has(imgId) && imageBadgeArchive.has(imgId)) {
          const archivedUrl = imageBadgeArchive.get(imgId)!
          imageBadgeMap.set(imgId, archivedUrl)
          if (!uploadedImages.value.includes(archivedUrl)) {
            uploadedImages.value.push(archivedUrl)
          }
        }
      }
      else if (attrs?.identity) {
        presentRefIds.add(attrs.identity)
        if (!referencedFiles.value.some(ref => getReferenceIdentity(ref) === attrs.identity) && attrs.referenceData) {
          try { referencedFiles.value.push(JSON.parse(attrs.referenceData)) }
          catch {}
        }
      }
    }
  })

  referencedFiles.value = referencedFiles.value.filter(ref => presentRefIds.has(getReferenceIdentity(ref)))

  for (const [id, dataUrl] of [...imageBadgeMap]) {
    if (!presentImgIds.has(id)) {
      imageBadgeMap.delete(id)
      const idx = uploadedImages.value.indexOf(dataUrl)
      if (idx > -1) {
        uploadedImages.value.splice(idx, 1)
        emit('imageRemove', idx)
      }
    }
  }
}

function syncFromEditor() {
  if (!editor.value) return
  userInput.value = serializeEditorContent()
  syncDataWithEditor()
  emitUpdate()
}

// ---- Badge 插入 ----

function buildRefBadgeAttrs(ref: FileReferenceAttachment): InlineBadgeAttrs {
  return {
    badgeType: ref.type,
    identity: getReferenceIdentity(ref),
    label: getReferenceDisplayLabel(ref),
    kind: getReferenceKindLabel(ref),
    serialized: getSerializedReferenceText(ref),
    referenceData: JSON.stringify(ref),
    imageBadgeId: null,
    title: ref.type === 'url' ? (ref.url || '') : (ref.path || ''),
  }
}

function insertReferenceBadge(ref: FileReferenceAttachment) {
  editorInsertBadge(editor.value, buildRefBadgeAttrs(ref))
  userInput.value = serializeEditorContent()
}

// ---- Badge 删除 ----

function removeReferenceByIdentity(identity: string) {
  const idx = referencedFiles.value.findIndex(item => getReferenceIdentity(item) === identity)
  if (idx > -1) referencedFiles.value.splice(idx, 1)
  editorRemoveBadge(editor.value, 'identity', identity)
  userInput.value = serializeEditorContent()
  emitUpdate()
}

// ---- 引用粘贴检测 ----

function tryParsePasteAsReference(text: string): boolean {
  const trimmed = text.trim()

  if (URL_PATTERN.test(trimmed)) {
    let name = trimmed
    try { name = new URL(trimmed).hostname + new URL(trimmed).pathname } catch {}
    const ref: FileReferenceAttachment = { type: 'url', url: trimmed, name }
    if (addFileReference(ref)) {
      insertReferenceBadge(ref)
      emitUpdate()
      return true
    }
    return false
  }

  if (PATH_PATTERN.test(trimmed)) {
    const ref: FileReferenceAttachment = {
      type: 'path',
      path: trimmed,
      name: trimmed.split(/[/\\]/).pop() || trimmed,
      kind: /\.[^/\\]+$/.test(trimmed) ? 'file' : 'directory',
    }
    if (addFileReference(ref)) {
      insertReferenceBadge(ref)
      emitUpdate()
      return true
    }
    return false
  }

  return false
}

// ============ 图片 Badge ============

let nextImageBadgeId = 0
const imageBadgeMap = new Map<string, string>()
const imageBadgeArchive = new Map<string, string>()

function addImageWithBadge(dataUrl: string, name: string): boolean {
  uploadedImages.value.push(dataUrl)
  imageNames.value.push(name)
  const badgeId = `img-${nextImageBadgeId++}`
  imageBadgeMap.set(badgeId, dataUrl)
  imageBadgeArchive.set(badgeId, dataUrl)

  editorInsertBadge(editor.value, {
    badgeType: 'image',
    identity: '',
    label: name,
    kind: '图片',
    serialized: `[image: ${name}]`,
    referenceData: '',
    imageBadgeId: badgeId,
    title: name,
  })

  userInput.value = serializeEditorContent()
  return true
}

function removeImageByBadgeId(badgeId: string) {
  const dataUrl = imageBadgeMap.get(badgeId)
  imageBadgeMap.delete(badgeId)

  if (dataUrl) {
    const idx = uploadedImages.value.indexOf(dataUrl)
    if (idx > -1) {
      uploadedImages.value.splice(idx, 1)
      imageNames.value.splice(idx, 1)
      emit('imageRemove', idx)
    }
  }

  editorRemoveBadge(editor.value, 'imageBadgeId', badgeId)
  userInput.value = serializeEditorContent()
  emitUpdate()
}

function removeImageBadgeByDataUrl(dataUrl: string) {
  for (const [id, url] of imageBadgeMap) {
    if (url === dataUrl) {
      imageBadgeMap.delete(id)
      editorRemoveBadge(editor.value, 'imageBadgeId', id)
      break
    }
  }
  userInput.value = serializeEditorContent()
}

// ============ 拖拽 & 粘贴 ============

function isImagePath(path: string): boolean {
  return /\.(png|jpe?g|gif|webp|bmp|svg|ico|tiff?)$/i.test(path)
}

async function setupDragDropListener() {
  try {
    const webview = getCurrentWebviewWindow()
    unlistenDragDrop = await webview.onDragDropEvent(async (event) => {
      switch (event.payload.type) {
        case 'enter':
        case 'over':
          isDragOver.value = true
          break
        case 'drop':
          isDragOver.value = false
          getCurrentWindow().setFocus().catch(() => {})
          await handleDroppedPaths(event.payload.paths)
          break
        case 'leave':
          isDragOver.value = false
          break
      }
    })
  }
  catch (error) {
    console.error('设置文件拖放监听器失败:', error)
  }
}

async function handleDroppedPaths(paths: string[]) {
  let hasChanges = false
  for (const rawPath of paths) {
    if (isImagePath(rawPath)) {
      try {
        const rawDataUrl: string = await invoke('read_image_file_as_data_url', { path: rawPath })
        const name = rawPath.split(/[/\\]/).pop() || 'image'
        let dataUrl = rawDataUrl
        let summary = ''
        if (imageCompressionEnabled.value) {
          const result = await compressImage(rawDataUrl)
          dataUrl = result.dataUrl
          summary = compressionSummary(result)
        }
        if (addImageWithBadge(dataUrl, name)) {
          message.success(summary ? `图片 ${name} 已添加 (${summary})` : `图片 ${name} 已添加`)
          hasChanges = true
        }
      }
      catch (error) {
        console.error('读取图片失败:', error)
        message.error(`图片读取失败: ${error}`)
      }
      continue
    }

    const hasExtension = /\.[^/\\]+$/.test(rawPath)
    const ref: FileReferenceAttachment = {
      type: 'path',
      path: rawPath,
      name: rawPath.split(/[/\\]/).pop() || rawPath,
      kind: hasExtension ? 'file' : 'directory',
    }

    if (addFileReference(ref)) {
      insertReferenceBadge(ref)
      hasChanges = true
    }
  }
  if (hasChanges) {
    editor.value?.commands.focus('end')
    emitUpdate()
  }
}

let _shiftHeld = false
function _onGlobalKeyDown(e: KeyboardEvent) { if (e.key === 'Shift') _shiftHeld = true }
function _onGlobalKeyUp(e: KeyboardEvent) { if (e.key === 'Shift') _shiftHeld = false }

function handleEditorPaste(event: ClipboardEvent): boolean {
  const clipboardData = event.clipboardData
  if (!clipboardData) return false

  const forcePlainText = _shiftHeld

  if (!forcePlainText) {
    let hasImage = false
    const items = Array.from(clipboardData.items)
    for (const item of items) {
      if (item.type.includes('image')) {
        hasImage = true
        const file = item.getAsFile()
        if (file) handleImageFiles([file])
      }
    }
    if (hasImage) return true
  }

  const plainText = clipboardData.getData('text/plain') || ''

  if (!forcePlainText && tryParsePasteAsReference(plainText)) return true

  if (plainText && editor.value) {
    const { state, view } = editor.value
    view.dispatch(state.tr.insertText(plainText))
    return true
  }

  return false
}

// 处理增强入口点击
function handleEnhanceClick() {
  if (props.submitting) {
    return
  }
  if (props.enhanceEnabled) {
    emit('enhance')
  }
  else {
    emit('openMcpToolsTab')
  }
}

async function handleImageFiles(files: FileList | File[]): Promise<void> {
  for (const file of files) {
    if (!file.type.startsWith('image/')) continue

    try {
      const rawBase64 = await fileToBase64(file)
      const name = file.name || `粘贴图片-${uploadedImages.value.length + 1}`
      let dataUrl = rawBase64
      let summary = ''
      if (imageCompressionEnabled.value) {
        const result = await compressImage(rawBase64)
        dataUrl = result.dataUrl
        summary = compressionSummary(result)
      }
      if (addImageWithBadge(dataUrl, name)) {
        message.success(summary ? `图片 ${name} 已添加 (${summary})` : `图片 ${name} 已添加`)
        emitUpdate()
      }
    }
    catch (error) {
      console.error('图片处理失败:', error)
      message.error(`图片 ${file.name} 处理失败`)
    }
  }
}

function fileToBase64(file: File): Promise<string> {
  return new Promise((resolve, reject) => {
    const reader = new FileReader()
    reader.onload = () => resolve(reader.result as string)
    reader.onerror = reject
    reader.readAsDataURL(file)
  })
}

function removeImage(index: number) {
  const dataUrl = uploadedImages.value[index]
  uploadedImages.value.splice(index, 1)
  imageNames.value.splice(index, 1)
  if (dataUrl) removeImageBadgeByDataUrl(dataUrl)
  emit('imageRemove', index)
  emitUpdate()
}

// 移除自定义图片预览功能，改用 Naive UI 的内置预览

// 加载自定义prompt配置
async function loadCustomPrompts() {
  try {
    const config = await invoke('get_custom_prompt_config')
    if (config) {
      const promptConfig = config as any
      customPrompts.value = (promptConfig.prompts || []).sort((a: CustomPrompt, b: CustomPrompt) => a.sort_order - b.sort_order)
      customPromptEnabled.value = promptConfig.enabled ?? true
      sortablePrompts.value = [...normalPrompts.value]

      if (customPrompts.value.length > 0) {
        initializeDragSort()
      }
    }
  }
  catch (error) {
    console.error('PopupInput: 加载自定义prompt失败:', error)
  }
}

// 处理自定义prompt点击
function handlePromptClick(prompt: CustomPrompt) {
  // 如果prompt内容为空或只有空格，直接清空输入框
  if (!prompt.content || prompt.content.trim() === '') {
    userInput.value = ''
    emitUpdate()
    return
  }

  if (userInput.value.trim()) {
    // 如果输入框有内容，显示插入选择对话框
    pendingPromptContent.value = prompt.content
    showInsertDialog.value = true
  }
  else {
    // 如果输入框为空，直接插入
    insertPromptContent(prompt.content)
  }
}

// 处理引用消息内容
function handleQuoteMessage(messageContent: string) {
  if (userInput.value.trim()) {
    // 输入框有内容，显示插入选择对话框
    pendingPromptContent.value = messageContent
    showInsertDialog.value = true
  }
  else {
    // 输入框为空，直接插入
    insertPromptContent(messageContent)
    message.success('原文内容已引用到输入框')
  }
}

// 插入prompt内容
function insertPromptContent(content: string, mode: 'replace' | 'append' = 'replace') {
  if (!editor.value) return

  if (mode === 'replace') {
    editor.value.commands.clearContent()
    referencedFiles.value = []
    uploadedImages.value = []
    imageBadgeMap.clear()
  }
  else {
    editor.value.commands.focus('end')
    const currentText = serializeEditorContent().trim()
    if (currentText) editor.value.commands.insertContent('\n\n')
  }

  editor.value.commands.insertContent(content)
  userInput.value = serializeEditorContent()
  emitUpdate()
  setTimeout(() => editor.value?.commands.focus(), 100)
}

// 处理插入模式选择
function handleInsertMode(mode: 'replace' | 'append') {
  insertPromptContent(pendingPromptContent.value, mode)
  showInsertDialog.value = false
  pendingPromptContent.value = ''
}

// 处理条件性prompt开关变化
async function handleConditionalToggle(promptId: string, value: boolean) {
  // 先更新本地状态
  const prompt = customPrompts.value.find(p => p.id === promptId)
  if (prompt) {
    prompt.current_state = value
  }

  // 保存到后端
  try {
    await invoke('update_conditional_prompt_state', {
      promptId,
      newState: value,
    })
    message.success('上下文追加状态已保存')
  }
  catch (error) {
    console.error('保存条件性prompt状态失败:', error)
    message.error(`保存设置失败: ${(error as any)?.message || String(error)}`)

    // 回滚本地状态
    if (prompt) {
      prompt.current_state = !value
    }
  }
}

// 生成条件性prompt的追加内容
function generateConditionalContent(): string {
  // 复用统一的上下文拼接逻辑，保持增强与输入一致
  const conditionalText = buildConditionalContext(conditionalPrompts.value)
  return conditionalText ? `\n\n${conditionalText}` : ''
}

// 移除拖拽排序初始化函数

async function initializeDragSort() {
  await nextTick()
  await nextTick()

  setTimeout(() => {
    let targetContainer = promptContainer.value

    if (!targetContainer) {
      targetContainer = document.querySelector('[data-prompt-container]') as HTMLElement
    }

    if (!targetContainer) {
      const containers = document.querySelectorAll('.flex.flex-wrap')
      for (let i = 0; i < containers.length; i++) {
        const container = containers[i] as HTMLElement
        if (container.querySelector('.sortable-item')) {
          targetContainer = container
          break
        }
      }
    }

    if (targetContainer) {
      promptContainer.value = targetContainer
      start()
    }
  }, 500)
}

// 保存prompt排序
async function savePromptOrder() {
  try {
    const promptIds = sortablePrompts.value.map(p => p.id)
    await invoke('update_custom_prompt_order', { promptIds })
    message.success('排序已保存')
  }
  catch (error) {
    console.error('保存排序失败:', error)
    message.error('保存排序失败')
    // 重新加载以恢复原始顺序
    loadCustomPrompts()
  }
}

// userInput 由 TipTap onUpdate 回调驱动，各入口已显式调用 emitUpdate

// 事件监听器引用
let unlistenCustomPromptUpdate: (() => void) | null = null
let unlistenWindowMove: (() => void) | null = null
let unlistenDragDrop: (() => void) | null = null

function fixIMEPosition() {
  const el = editor.value?.view.dom
  if (el && document.activeElement === el) {
    el.blur()
    setTimeout(() => editor.value?.commands.focus(), 10)
  }
}

// 设置窗口移动监听器
async function setupWindowMoveListener() {
  try {
    const webview = getCurrentWebviewWindow()
    // 监听窗口移动事件
    unlistenWindowMove = await webview.onMoved(() => {
      // 窗口移动后修复输入法位置
      fixIMEPosition()
    })
  }
  catch (error) {
    console.error('设置窗口移动监听器失败:', error)
  }
}

// ============ TipTap Editor ============

const editor = useEditor({
  extensions: [
    StarterKit.configure({
      heading: false,
      blockquote: false,
      codeBlock: false,
      bulletList: false,
      orderedList: false,
      listItem: false,
      horizontalRule: false,
      bold: false,
      italic: false,
      strike: false,
      code: false,
    }),
    InlineBadge,
  ],
  content: '',
  editable: !props.submitting,
  editorProps: {
    attributes: {
      'data-guide': 'popup-input',
    },
    handlePaste: (_view, event) => {
      return handleEditorPaste(event)
    },
  },
  onUpdate: () => {
    syncFromEditor()
  },
})

const isEditorEmpty = computed(() => !editor.value || editor.value.isEmpty)

const placeholderText = computed(() => {
  return hasOptions.value
    ? `您可以在这里添加补充说明... (支持粘贴图片 ${pasteShortcut.value}、拖拽文件/文件夹、粘贴路径或 URL)`
    : `请输入您的回复... (支持粘贴图片 ${pasteShortcut.value}、拖拽文件/文件夹、粘贴路径或 URL)`
})

watch(() => props.submitting, (val) => {
  editor.value?.setEditable(!val)
})

// 组件挂载时加载自定义prompt
onMounted(async () => {
  try {
    imageCompressionEnabled.value = await invoke('get_image_compression_enabled') as boolean
  }
  catch { /* 默认 true */ }
  await loadCustomPrompts()

  // 加载 MCP 工具配置（用于检查关联工具状态）
  await loadMcpTools()

  // 监听自定义prompt更新事件
  unlistenCustomPromptUpdate = await listen('custom-prompt-updated', () => {
    loadCustomPrompts()
  })
  // 设置窗口移动监听器
  setupWindowMoveListener()
  // 设置文件拖放监听器（Tauri 原生）
  setupDragDropListener()

  window.addEventListener('keydown', _onGlobalKeyDown, true)
  window.addEventListener('keyup', _onGlobalKeyUp, true)
})

onUnmounted(() => {
  // 清理事件监听器
  if (unlistenCustomPromptUpdate) {
    unlistenCustomPromptUpdate()
  }
  // 清理窗口移动监听器
  if (unlistenWindowMove) {
    unlistenWindowMove()
  }
  // 清理文件拖放监听器
  if (unlistenDragDrop) {
    unlistenDragDrop()
  }

  // 停止拖拽功能
  stop()

  window.removeEventListener('keydown', _onGlobalKeyDown, true)
  window.removeEventListener('keyup', _onGlobalKeyUp, true)
})

function reset() {
  editor.value?.commands.clearContent()
  userInput.value = ''
  selectedOptions.value = []
  uploadedImages.value = []
  imageNames.value = []
  referencedFiles.value = []
  imageBadgeMap.clear()
  imageBadgeArchive.clear()
  nextImageBadgeId = 0
  emitUpdate()
}

function updateData(data: { userInput?: string, selectedOptions?: string[], draggedImages?: string[], referencedFiles?: FileReferenceAttachment[] }) {
  if (data.userInput !== undefined && editor.value) {
    editor.value.commands.setContent(data.userInput)
    userInput.value = data.userInput
  }
  if (data.selectedOptions !== undefined) {
    selectedOptions.value = data.selectedOptions
  }
  if (data.draggedImages !== undefined) {
    uploadedImages.value = data.draggedImages
  }
  if (data.referencedFiles !== undefined) {
    referencedFiles.value = data.referencedFiles
    if (editor.value && data.referencedFiles.length > 0) {
      for (const ref of data.referencedFiles) {
        insertReferenceBadge(ref)
      }
    }
  }

  emitUpdate()
}

// 中文注释：暴露原始输入与附加上下文，供本地增强链路精确组装提示词
function getRawUserInput() {
  return userInput.value
}

function getConditionalContext() {
  return generateConditionalContent()
}

// 移除了文件选择和测试图片功能

// 暴露方法给父组件
defineExpose({
  reset,
  canSubmit,
  canEnhance,
  statusText,
  updateData,
  handleQuoteMessage,
  getRawUserInput,
  getConditionalContext,
})
</script>

<template>
  <div class="space-y-2">
    <!-- 预定义选项 -->
    <div v-if="!loading && hasOptions" class="space-y-2" data-guide="predefined-options">
      <h4 class="text-sm font-medium text-on-surface m-0 py-1">
        请选择选项
      </h4>
      <div class="flex flex-col gap-1">
        <div
          v-for="(option, index) in request!.predefined_options"
          :key="`option-${index}`"
          class="px-3 py-1.5 bg-container-secondary rounded-[3px] cursor-pointer hover:bg-black-200 transition-colors"
          @click="handleOptionToggle(option)"
        >
          <n-checkbox
            :value="option"
            :checked="selectedOptions.includes(option)"
            :disabled="submitting"
            size="small"
            @update:checked="(checked: boolean) => handleOptionChange(option, checked)"
            @click.stop
          >
            {{ option }}
          </n-checkbox>
        </div>
      </div>
    </div>

    <!-- 文本输入区域 -->
    <Teleport to="#floating-input-target" :disabled="!isFloating" defer>
    <div v-if="!loading">
      <div
        class="transition-all duration-300 ease-[cubic-bezier(0.25,0.8,0.25,1)]"
        :class="[
          isFloating
            ? 'bg-surface/85 backdrop-blur-xl shadow-[0_-8px_30px_rgba(0,0,0,0.12)] border-t border-white/10 pb-2 pt-2 px-3 space-y-2'
            : 'relative space-y-2',
        ]"
      >
        <!-- 标题栏 & 切换按钮 -->
        <div class="flex items-center justify-between mb-2">
          <div
            class="flex items-center gap-1"
            :class="isFloating ? 'cursor-pointer select-none hover:opacity-80 transition-opacity' : ''"
            :title="isFloating ? (isFloatingExpanded ? '折叠面板' : '展开模板和上下文') : undefined"
            @click="isFloating && toggleFloatingExpanded()"
          >
            <h4 class="text-sm font-medium text-on-surface m-0 py-1">
              {{ hasOptions ? '补充说明 (可选)' : '请输入您的回复' }}
            </h4>
            <div
              v-if="isFloating"
              class="w-3 h-3 transition-transform duration-200 opacity-60"
              :class="isFloatingExpanded ? 'i-carbon-chevron-up' : 'i-carbon-chevron-down'"
            />
          </div>
          <n-button
            text
            size="tiny"
            class="opacity-70 hover:opacity-100 transition-opacity"
            :title="isFloating ? '取消悬浮 (跟随底部)' : '开启悬浮 (固定底部)'"
            @click="toggleFloating"
          >
            <template #icon>
              <div
                class="transition-transform duration-300"
                :class="[
                  isFloating ? 'i-carbon-pin-filled text-primary-500 rotate-0' : 'i-carbon-pin text-on-surface-secondary -rotate-45',
                ]"
              />
            </template>
          </n-button>
        </div>

        <!-- 自定义prompt按钮区域 -->
        <div v-if="customPromptEnabled && customPrompts.length > 0" v-show="!isFloating || isFloatingExpanded" class="space-y-2" data-guide="custom-prompts">
          <div class="text-xs text-on-surface-secondary flex items-center gap-2">
            <div class="i-carbon-bookmark w-3 h-3 text-primary-500" />
            <span>快捷模板 (拖拽调整顺序):</span>
          </div>
          <div
            ref="promptContainer"
            data-prompt-container
            class="flex flex-wrap gap-2"
          >
            <div
              v-for="prompt in sortablePrompts"
              :key="prompt.id"
              :title="prompt.description || (prompt.content.trim() ? prompt.content : '清空输入框')"
              class="inline-flex items-center gap-1 px-2 py-1 text-xs bg-container-secondary hover:bg-black-200 rounded-[3px] transition-all duration-200 select-none border border-gray-600 text-on-surface sortable-item"
            >
              <!-- 拖拽手柄 -->
              <div class="drag-handle cursor-move p-0.5 rounded-[3px] hover:bg-black-200 transition-colors">
                <div class="i-carbon-drag-horizontal w-3 h-3 text-on-surface-secondary" />
              </div>

              <!-- 按钮内容 -->
              <div
                class="inline-flex items-center cursor-pointer"
                @click="handlePromptClick(prompt)"
              >
                <span>{{ prompt.name }}</span>
              </div>
            </div>
          </div>
        </div>

        <!-- 上下文追加区域 -->
        <div v-if="customPromptEnabled && conditionalPrompts.length > 0" v-show="!isFloating || isFloatingExpanded" class="space-y-2" data-guide="context-append">
          <div class="text-xs text-on-surface-secondary flex items-center gap-2">
            <div class="i-carbon-settings-adjust w-3 h-3 text-primary-500" />
            <span>上下文追加:</span>
          </div>
          <div class="grid grid-cols-3 gap-1">
            <div
              v-for="prompt in conditionalPrompts"
              :key="prompt.id"
              class="flex items-center justify-between p-1.5 bg-container-secondary rounded-[3px] border border-gray-600 transition-colors text-xs"
              :class="[
                isMcpToolEnabled(prompt.linked_mcp_tool) ? 'hover:bg-black-200' : 'opacity-50 cursor-not-allowed',
              ]"
            >
              <div class="flex-1 min-w-0 mr-1">
                <div class="text-xs text-on-surface truncate font-medium" :title="prompt.condition_text || prompt.name">
                  {{ prompt.condition_text || prompt.name }}
                </div>
              </div>
              <!-- 使用 n-tooltip 包裹开关，当 MCP 工具未启用时显示提示 -->
              <n-tooltip :disabled="isMcpToolEnabled(prompt.linked_mcp_tool) || !prompt.linked_mcp_tool">
                <template #trigger>
                  <n-switch
                    :value="prompt.current_state ?? false"
                    size="small"
                    :disabled="!isMcpToolEnabled(prompt.linked_mcp_tool)"
                    @update:value="(value: boolean) => handleConditionalToggle(prompt.id, value)"
                  />
                </template>
                请先在设置中开启「{{ getMcpToolName(prompt.linked_mcp_tool) }}」MCP 工具
              </n-tooltip>
            </div>
          </div>
        </div>

        <!-- 提示词增强入口 -->
        <div class="flex items-center justify-between text-xs" :class="isFloating ? 'my-1' : 'my-2'">
          <div class="flex items-center gap-2 text-on-surface-secondary">
            <div class="i-carbon-magic-wand w-3 h-3 text-primary-500" />
            <span>{{ enhanceEnabled ? '将当前文本发送给本地 AI 做结构化增强' : '提示词增强未启用' }}</span>
          </div>
          <n-button
            size="tiny"
            :type="enhanceEnabled ? 'info' : 'warning'"
            secondary
            :disabled="submitting || (enhanceEnabled && !canEnhance)"
            @click="handleEnhanceClick"
          >
            <template #icon>
              <div :class="enhanceEnabled ? 'i-carbon-magic-wand' : 'i-carbon-launch'" />
            </template>
            {{ enhanceEnabled ? '本地增强' : '启用增强' }}
          </n-button>
        </div>

        <!-- 图片预览（嵌入输入框上方，floating 时跟随） -->
        <div v-if="uploadedImages.length > 0" class="flex items-center gap-1 flex-wrap">
          <n-image-group>
            <div
              v-for="(image, index) in uploadedImages"
              :key="`image-${index}`"
              class="relative group"
            >
              <n-image
                :src="image"
                width="40"
                height="40"
                object-fit="cover"
                class="!rounded-[3px] border border-border hover:border-primary transition-colors cursor-pointer"
              />
              <n-button
                class="absolute -top-1.5 -right-1.5 z-10 opacity-0 group-hover:opacity-100 transition-opacity"
                size="tiny"
                type="error"
                circle
                @click="removeImage(index)"
              >
                <template #icon>
                  <div class="i-carbon-close w-2.5 h-2.5" />
                </template>
              </n-button>
            </div>
          </n-image-group>
        </div>

        <!-- TipTap 输入框 -->
        <div
          class="popup-input-shell"
          :class="{ 'popup-input-shell-dragover': isDragOver }"
          @click="editor?.commands.focus()"
        >
          <!-- 拖拽悬停提示 -->
          <div v-if="isDragOver" class="popup-drag-overlay">
            <div class="i-carbon-upload w-6 h-6 text-primary-400" />
            <span class="text-xs text-primary-400">释放以添加引用</span>
          </div>

          <!-- placeholder 覆盖层 -->
          <div v-if="isEditorEmpty" class="popup-placeholder">
            {{ placeholderText }}
          </div>

          <EditorContent :editor="editor" />
        </div>
      </div>
    </div>
    </Teleport>

    <!-- 插入模式选择对话框 -->
    <AppModal v-model:show="showInsertDialog" preset="dialog" title="插入模式选择">
      <template #header>
        <div class="flex items-center gap-2">
          <div class="i-carbon-text-creation w-4 h-4" />
          <span>插入Prompt</span>
        </div>
      </template>
      <div class="space-y-4">
        <p class="text-sm text-on-surface-secondary">
          输入框中已有内容，请选择插入模式：
        </p>
        <div class="bg-container-secondary p-3 rounded-[3px] text-sm max-h-40 overflow-y-auto">
          {{ pendingPromptContent }}
        </div>
      </div>
      <template #action>
        <n-space justify="end">
          <n-button size="small" @click="showInsertDialog = false">
            取消
          </n-button>
          <n-button type="warning" size="small" @click="handleInsertMode('replace')">
            替换内容
          </n-button>
          <n-button type="primary" size="small" @click="handleInsertMode('append')">
            追加内容
          </n-button>
        </n-space>
      </template>
    </AppModal>
  </div>
</template>

<style scoped>
/* Sortable.js 拖拽样式 */
.sortable-ghost {
  opacity: 0.5;
  transform: scale(0.95);
}

.sortable-chosen {
  cursor: grabbing !important;
}

.sortable-drag {
  opacity: 0.8;
  transform: rotate(5deg);
}

/* TipTap 输入框 - 精确模拟 Naive UI n-input 样式 */
.popup-input-shell {
  position: relative;
  border: 1px solid var(--color-border);
  border-radius: 3px;
  overflow: hidden;
  background-color: var(--color-container-secondary);
  transition: border-color 0.3s var(--n-bezier, cubic-bezier(.4, 0, .2, 1)),
              box-shadow 0.3s var(--n-bezier, cubic-bezier(.4, 0, .2, 1)),
              background-color 0.3s var(--n-bezier, cubic-bezier(.4, 0, .2, 1));
}

.popup-input-shell:hover {
  border-color: var(--color-primary-hover, var(--color-primary));
}

.popup-input-shell:focus-within {
  border-color: var(--color-primary-hover, var(--color-primary));
  box-shadow: 0 0 0 2px color-mix(in srgb, var(--color-primary) 20%, transparent);
  background-color: color-mix(in srgb, var(--color-primary) 2%, var(--color-container-secondary));
}

.popup-input-shell-dragover {
  border-color: var(--color-primary);
  background-color: color-mix(in srgb, var(--color-primary) 4%, var(--color-container-secondary));
}

.popup-drag-overlay {
  position: absolute;
  inset: 0;
  z-index: 10;
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  gap: 0.25rem;
  background: color-mix(in srgb, var(--color-primary) 8%, transparent);
  border-radius: inherit;
  pointer-events: none;
}

.popup-placeholder {
  position: absolute;
  top: 4px;
  left: 10px;
  right: 10px;
  color: var(--color-surface-500);
  pointer-events: none;
  font-size: 14px;
  line-height: 1.6;
  z-index: 1;
  white-space: pre-wrap;
  word-break: break-word;
}

:deep(.tiptap) {
  width: 100%;
  min-height: 80px;
  max-height: 180px;
  padding: 4px 10px;
  overflow-y: auto;
  overflow-x: hidden;
  outline: none;
  font-size: 14px;
  line-height: 1.6;
  color: var(--color-on-surface);
  background: transparent;
  border: none;
  box-sizing: border-box;
  word-break: break-word;
  white-space: pre-wrap;
  scrollbar-width: thin;
  scrollbar-color: color-mix(in srgb, var(--color-on-surface) 25%, transparent) transparent;
}

:deep(.tiptap:focus) {
  outline: none;
}

:deep(.tiptap p) {
  margin: 0;
}

:deep(.tiptap ::selection) {
  background: color-mix(in srgb, var(--color-primary) 30%, transparent);
}

/* 内联引用 badge */
:deep(.popup-inline-reference) {
  display: inline-flex;
  max-width: 15rem;
  position: relative;
  align-items: center;
  gap: 0.25rem;
  margin: 0 0.15rem;
  padding: 0.1rem 0.35rem;
  border: 1px solid var(--color-surface-300);
  border-radius: 4px;
  background: var(--color-surface-200);
  color: var(--color-on-surface);
  vertical-align: text-bottom;
  user-select: none;
  cursor: default;
  font-size: 12px;
  line-height: 1.2;
  transition: border-color 0.2s, background 0.2s;
}

:deep(.popup-inline-reference:hover) {
  border-color: var(--color-surface-400);
  background: var(--color-surface-300);
}

:deep(.popup-inline-reference-icon-slot) {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 14px;
  height: 14px;
  flex-shrink: 0;
  position: relative;
}

:deep(.popup-inline-reference-kind) {
  width: 12px;
  height: 12px;
  color: var(--color-on-surface-secondary);
  transition: opacity 0.15s;
}

:deep(.popup-inline-reference-delete) {
  position: absolute;
  inset: 0;
  display: flex;
  align-items: center;
  justify-content: center;
  opacity: 0;
  cursor: pointer;
  pointer-events: auto;
  color: var(--color-on-surface-secondary);
  transition: opacity 0.15s, color 0.15s;
}

:deep(.popup-inline-reference:hover .popup-inline-reference-kind) {
  opacity: 0;
}

:deep(.popup-inline-reference:hover .popup-inline-reference-delete) {
  opacity: 1;
}

:deep(.popup-inline-reference-delete:hover) {
  color: var(--color-error);
}

:deep(.popup-inline-reference-label) {
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  font-size: 12px;
}

:deep(.popup-inline-reference-label.cursor-pointer:hover) {
  text-decoration: underline;
}

:deep(.popup-inline-reference.badge-in-selection) {
  background: var(--color-primary);
  border-color: var(--color-primary);
  color: #fff;
}

:deep(.badge-popover) {
  position: absolute;
  top: calc(100% + 4px);
  left: 50%;
  transform: translateX(-50%);
  display: inline-flex;
  align-items: center;
  gap: 0;
  padding: 2px;
  border-radius: 6px;
  border: 1px solid var(--color-surface-300);
  background: var(--color-surface-200);
  box-shadow: 0 4px 12px color-mix(in srgb, var(--color-on-surface) 15%, transparent);
  z-index: 100;
  white-space: nowrap;
  font-size: 11px;
}

:deep(.badge-popover-item) {
  display: inline-flex;
  align-items: center;
  gap: 4px;
  padding: 3px 8px;
  border-radius: 4px;
  cursor: pointer;
  color: var(--color-on-surface);
  transition: background 0.15s;
}

:deep(.badge-popover-item:hover) {
  background: var(--color-surface-300);
}

:deep(.badge-popover-divider) {
  width: 1px;
  height: 14px;
  background: var(--color-surface-300);
  flex-shrink: 0;
}
</style>
