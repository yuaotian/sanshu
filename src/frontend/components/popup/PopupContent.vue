<script setup lang="ts">
import type { McpRequest } from '../../types/popup'
import { invoke } from '@tauri-apps/api/core'
import { writeImage } from '@tauri-apps/plugin-clipboard-manager'
import { useMessage } from 'naive-ui'
import { computed, nextTick, onBeforeUnmount, onMounted, ref, watch } from 'vue'
import { safeBase64Decode, useMarkdown } from '../../composables/useMarkdown'

const props = withDefaults(defineProps<Props>(), {
  loading: false,
  currentTheme: 'dark',
})

const emit = defineEmits<Emits>()

const { renderMarkdown, loadHljsTheme } = useMarkdown()
const message = useMessage()
const markdownRoot = ref<HTMLElement | null>(null)
const showRawMarkdown = ref(false)
const imagePreview = ref<ImagePreviewState | null>(null)

let mermaidRenderSeq = 0
let resizeObserver: ResizeObserver | null = null
let mermaidInstance: Awaited<typeof import('mermaid')>['default'] | null = null

const currentThemeIsLight = computed(() => props.currentTheme === 'light')
const markdownThemeClass = computed(() => currentThemeIsLight.value ? 'theme-light' : 'theme-dark')

const renderedMarkdown = computed(() => {
  if (!props.request?.message) return ''
  return renderMarkdown(props.request.message)
})

// 预处理引用内容，移除增强prompt格式标记
function preprocessQuoteContent(content: string): string {
  let processedContent = content

  const markersToRemove = [
    /### BEGIN RESPONSE ###\s*/gi,
    /Here is an enhanced version of the original instruction that is more specific and clear:\s*/gi,
    /<augment-enhanced-prompt>\s*/gi,
    /<\/augment-enhanced-prompt>\s*/gi,
    /### END RESPONSE ###\s*/gi,
  ]

  markersToRemove.forEach((marker) => {
    processedContent = processedContent.replace(marker, '')
  })

  processedContent = processedContent
    .replace(/\n\s*\n\s*\n/g, '\n\n')
    .trim()

  return processedContent
}

// 引用消息内容
function quoteMessage() {
  if (props.request?.message) {
    const processedContent = preprocessQuoteContent(props.request.message)
    emit('quoteMessage', processedContent)
  }
}

function getMermaidTheme() {
  return currentThemeIsLight.value ? 'default' : 'dark'
}

async function getMermaidInstance() {
  if (!mermaidInstance) {
    mermaidInstance = (await import('mermaid')).default
  }
  return mermaidInstance
}

async function initializeMermaid() {
  const mermaid = await getMermaidInstance()
  mermaid.initialize({
    startOnLoad: false,
    theme: getMermaidTheme(),
    securityLevel: 'strict',
    logLevel: 'error',
    flowchart: {
      htmlLabels: false,
      curve: 'basis',
    },
  })
  return mermaid
}

async function prepareRenderedMarkdown() {
  if (showRawMarkdown.value) return
  await nextTick()
  await renderMermaidBlocks()
  enhanceMarkdownImages()
}

async function renderMermaidBlocks() {
  const root = markdownRoot.value
  if (!root) return

  const seq = ++mermaidRenderSeq
  const wrappers = Array.from(root.querySelectorAll<HTMLElement>('.mermaid-block-wrapper'))
  if (wrappers.length === 0) return
  const mermaid = await initializeMermaid()

  for (const [index, wrapper] of wrappers.entries()) {
    if (seq !== mermaidRenderSeq) return
    const renderTarget = wrapper.querySelector<HTMLElement>('.mermaid-render')
    const encodedCode = wrapper.getAttribute('data-diagram-code')
    if (!renderTarget || !encodedCode) continue

    try {
      const code = safeBase64Decode(encodedCode)
      const diagramId = `popup-mermaid-${props.request?.id || 'request'}-${index}-${seq}`.replace(/[^a-zA-Z0-9_-]/g, '-')
      const { svg, bindFunctions } = await mermaid.render(diagramId, code)
      if (seq !== mermaidRenderSeq) return
      renderTarget.innerHTML = svg
      renderTarget.classList.remove('mermaid-error')
      renderTarget.setAttribute('data-zoom', '1')
      delete renderTarget.dataset.baseWidth
      delete renderTarget.dataset.baseHeight
      applyMermaidZoom(wrapper, 1)
      bindFunctions?.(renderTarget)
    }
    catch (error) {
      renderTarget.classList.add('mermaid-error')
      renderTarget.textContent = `流程图渲染失败：${String(error)}`
    }
  }
}

function enhanceMarkdownImages() {
  const root = markdownRoot.value
  if (!root) return

  const images = Array.from(root.querySelectorAll<HTMLImageElement>('img'))
  images.forEach((image, index) => {
    if (image.closest('.markdown-image-wrapper')) return

    const wrapper = document.createElement('span')
    wrapper.className = 'markdown-image-wrapper'
    wrapper.dataset.imageIndex = String(index)

    const toolbar = document.createElement('span')
    toolbar.className = 'markdown-image-toolbar'
    toolbar.innerHTML = [
      '<button class="markdown-image-action markdown-image-preview" title="查看原图"><div class="i-carbon-view" style="width:14px;height:14px;display:block;"></div></button>',
      '<button class="markdown-image-action markdown-image-copy" title="复制原图"><div class="i-carbon-copy" style="width:14px;height:14px;display:block;"></div></button>',
    ].join('')

    image.parentNode?.insertBefore(wrapper, image)
    wrapper.appendChild(image)
    wrapper.appendChild(toolbar)
  })
}

async function handleMermaidButton(button: HTMLButtonElement, wrapper: HTMLElement) {
  if (button.classList.contains('mermaid-source-toggle')) {
    wrapper.classList.toggle('show-source')
    return
  }

  if (button.classList.contains('mermaid-zoom-in')) {
    applyMermaidZoom(wrapper, getMermaidZoom(wrapper) + 0.15)
    return
  }

  if (button.classList.contains('mermaid-zoom-out')) {
    applyMermaidZoom(wrapper, getMermaidZoom(wrapper) - 0.15)
    return
  }

  if (button.classList.contains('mermaid-zoom-reset')) {
    applyMermaidZoom(wrapper, 1)
    return
  }

  if (button.classList.contains('mermaid-copy-image')) {
    await copyMermaidImage(wrapper, button)
  }
}

function getMermaidZoom(wrapper: HTMLElement) {
  const renderTarget = wrapper.querySelector<HTMLElement>('.mermaid-render')
  return Number(renderTarget?.getAttribute('data-zoom') || '1')
}

function applyMermaidZoom(wrapper: HTMLElement, zoom: number) {
  const renderTarget = wrapper.querySelector<HTMLElement>('.mermaid-render')
  const scale = Math.min(3, Math.max(0.4, Number(zoom.toFixed(2))))
  if (!renderTarget) return

  renderTarget.setAttribute('data-zoom', String(scale))
  renderTarget.style.transform = ''
  const svg = renderTarget.querySelector<SVGSVGElement>('svg')
  if (svg) {
    const baseWidth = Number(renderTarget.dataset.baseWidth || svg.getBoundingClientRect().width || svg.clientWidth || 1)
    const baseHeight = Number(renderTarget.dataset.baseHeight || svg.getBoundingClientRect().height || svg.clientHeight || 1)
    renderTarget.dataset.baseWidth = String(baseWidth)
    renderTarget.dataset.baseHeight = String(baseHeight)
    renderTarget.style.width = `${baseWidth * scale}px`
    renderTarget.style.height = `${baseHeight * scale}px`
    svg.style.width = `${baseWidth * scale}px`
    svg.style.height = `${baseHeight * scale}px`
  }
}

async function copyMermaidImage(wrapper: HTMLElement, triggerEl: HTMLElement) {
  const svg = wrapper.querySelector<SVGSVGElement>('.mermaid-render svg')
  if (!svg) {
    message.warning('流程图尚未渲染完成')
    return
  }

  try {
    const bytes = await svgToPngBytes(svg)
    await writeImage(bytes)
    showTemporaryCheck(triggerEl)
    message.success('流程图图片已复制到剪贴板')
  }
  catch {
    const encodedCode = wrapper.getAttribute('data-diagram-code')
    if (encodedCode) {
      await navigator.clipboard.writeText(safeBase64Decode(encodedCode))
      message.warning('图片复制失败，已复制流程图源码')
    }
    else {
      message.error('复制失败')
    }
  }
}

async function handleMarkdownImageButton(button: HTMLButtonElement, image: HTMLImageElement) {
  if (button.classList.contains('markdown-image-preview')) {
    openImagePreview(image)
    return
  }

  if (button.classList.contains('markdown-image-copy')) {
    await copyMarkdownImage(image, button)
  }
}

function openImagePreview(image: HTMLImageElement) {
  imagePreview.value = {
    src: image.currentSrc || image.src,
    alt: image.alt || 'Markdown 图片',
    zoom: 1,
  }
}

function closeImagePreview() {
  imagePreview.value = null
}

function adjustPreviewZoom(delta: number) {
  if (!imagePreview.value) return
  imagePreview.value.zoom = Math.min(4, Math.max(0.25, Number((imagePreview.value.zoom + delta).toFixed(2))))
}

function resetPreviewZoom() {
  if (imagePreview.value) {
    imagePreview.value.zoom = 1
  }
}

async function copyPreviewImage() {
  if (!imagePreview.value) return

  try {
    const bytes = await imageUrlToPngBytes(imagePreview.value.src)
    await writeImage(bytes)
    message.success('图片已复制到剪贴板')
  }
  catch {
    await navigator.clipboard.writeText(imagePreview.value.src)
    message.warning('图片复制失败，已复制图片地址')
  }
}

async function copyMarkdownImage(image: HTMLImageElement, triggerEl: HTMLElement) {
  const src = image.currentSrc || image.src
  try {
    const bytes = await imageUrlToPngBytes(src)
    await writeImage(bytes)
    showTemporaryCheck(triggerEl)
    message.success('图片已复制到剪贴板')
  }
  catch {
    await navigator.clipboard.writeText(src)
    message.warning('图片复制失败，已复制图片地址')
  }
}

async function imageUrlToPngBytes(src: string) {
  const image = await loadImage(src)
  return imageToPngBytes(image)
}

function loadImage(src: string): Promise<HTMLImageElement> {
  return new Promise((resolve, reject) => {
    const image = new Image()
    image.crossOrigin = 'anonymous'
    image.onload = () => resolve(image)
    image.onerror = () => reject(new Error('图片加载失败'))
    image.src = src
  })
}

async function svgToPngBytes(svg: SVGSVGElement) {
  const clonedSvg = svg.cloneNode(true) as SVGSVGElement
  clonedSvg.setAttribute('xmlns', 'http://www.w3.org/2000/svg')
  const rect = svg.getBoundingClientRect()
  const width = Math.max(1, Math.ceil(rect.width))
  const height = Math.max(1, Math.ceil(rect.height))
  const svgText = new XMLSerializer().serializeToString(clonedSvg)
  const blob = new Blob([svgText], { type: 'image/svg+xml;charset=utf-8' })
  const url = URL.createObjectURL(blob)

  try {
    const image = await loadImage(url)
    return imageToPngBytes(image, width, height)
  }
  finally {
    URL.revokeObjectURL(url)
  }
}

function imageToPngBytes(image: HTMLImageElement, targetWidth?: number, targetHeight?: number): Promise<Uint8Array> {
  return new Promise((resolve, reject) => {
    const canvas = document.createElement('canvas')
    canvas.width = targetWidth || image.naturalWidth || image.width
    canvas.height = targetHeight || image.naturalHeight || image.height
    const ctx = canvas.getContext('2d')
    if (!ctx) {
      reject(new Error('Canvas 初始化失败'))
      return
    }

    ctx.fillStyle = '#ffffff'
    ctx.fillRect(0, 0, canvas.width, canvas.height)
    ctx.drawImage(image, 0, 0, canvas.width, canvas.height)
    canvas.toBlob(async (blob) => {
      if (!blob) {
        reject(new Error('图片转换失败'))
        return
      }
      resolve(new Uint8Array(await blob.arrayBuffer()))
    }, 'image/png')
  })
}

function showTemporaryCheck(triggerEl: HTMLElement) {
  const icon = triggerEl.querySelector('div')
  if (!icon) return

  const oldClass = icon.className
  const oldStyle = icon.getAttribute('style') || ''
  icon.className = 'i-carbon-checkmark'
  icon.setAttribute('style', 'width:14px;height:14px;display:block;color:#22c55e;')
  setTimeout(() => {
    icon.className = oldClass
    icon.setAttribute('style', oldStyle)
  }, 1600)
}

// 复制原始 Markdown 内容到剪贴板
async function copyRawMarkdown() {
  const content = props.request?.message
  if (!content) {
    message.warning('暂无内容可复制')
    return
  }
  try {
    await navigator.clipboard.writeText(content)
    message.success('Markdown 原文已复制')
  }
  catch {
    message.error('复制失败')
  }
}

// 事件委托 — 处理 markdown 内容区域的点击
async function handleMarkdownClick(e: MouseEvent) {
  const target = e.target as HTMLElement

  // 代码块复制按钮
  const copyBtn = target.closest('.code-block-copy') as HTMLElement | null
  if (copyBtn) {
    e.stopPropagation()
    e.preventDefault()
    const encodedCode = copyBtn.getAttribute('data-code')
    if (!encodedCode) return
    try {
      const code = safeBase64Decode(encodedCode)
      await navigator.clipboard.writeText(code)
      // 切换图标为 checkmark
      const icon = copyBtn.querySelector('div')
      if (icon) {
        const oldClass = icon.className
        icon.className = 'i-carbon-checkmark'
        icon.style.cssText = 'width:14px;height:14px;display:block;color:#22c55e;'
        setTimeout(() => {
          icon.className = oldClass
          icon.style.cssText = 'width:14px;height:14px;display:block;'
        }, 2000)
      }
      message.success('代码已复制到剪贴板')
    }
    catch {
      message.error('复制失败')
    }
    return
  }

  // 代码块运行按钮
  const runBtn = target.closest('.code-block-run') as HTMLElement | null
  if (runBtn) {
    e.stopPropagation()
    e.preventDefault()
    const lang = runBtn.getAttribute('data-lang') || ''
    const encodedCode = runBtn.getAttribute('data-code')
    if (!encodedCode) return
    const code = safeBase64Decode(encodedCode)
    await handleCodeExecution(lang, code, runBtn)
    return
  }

  const mermaidWrapper = target.closest('.mermaid-block-wrapper') as HTMLElement | null
  if (mermaidWrapper) {
    const button = target.closest('button') as HTMLButtonElement | null
    if (button) {
      e.stopPropagation()
      e.preventDefault()
      await handleMermaidButton(button, mermaidWrapper)
      return
    }
  }

  const imageButton = target.closest('.markdown-image-action') as HTMLButtonElement | null
  if (imageButton) {
    const imageWrapper = imageButton.closest('.markdown-image-wrapper') as HTMLElement | null
    const image = imageWrapper?.querySelector('img') as HTMLImageElement | null
    if (image) {
      e.stopPropagation()
      e.preventDefault()
      await handleMarkdownImageButton(imageButton, image)
      return
    }
  }

  const markdownImage = target.closest('.markdown-image-wrapper img, .markdown-content img') as HTMLImageElement | null
  if (markdownImage && !target.closest('.markdown-image-action')) {
    e.stopPropagation()
    e.preventDefault()
    openImagePreview(markdownImage)
    return
  }

  // 内联代码复制
  const inlineCode = target.closest('.markdown-content p code, .markdown-content li code') as HTMLElement | null
  if (inlineCode) {
    try {
      await navigator.clipboard.writeText(inlineCode.textContent || '')
      message.success('代码已复制到剪贴板')
    }
    catch {
      message.error('复制失败')
    }
  }
}

// 代码执行处理
async function handleCodeExecution(lang: string, code: string, triggerEl: HTMLElement) {
  const wrapper = triggerEl.closest('.code-block-wrapper')
  if (!wrapper) return

  // 检查是否已有输出面板
  let outputPanel = wrapper.querySelector('.code-execution-output') as HTMLElement | null
  if (outputPanel) {
    outputPanel.remove()
    return
  }

  // 创建输出面板
  outputPanel = document.createElement('div')
  outputPanel.className = 'code-execution-output'
  outputPanel.innerHTML = '<div class="output-header"><span>运行中...</span></div><pre style="margin:0;padding:0;border:none;background:none;font-size:inherit;color:inherit;">等待执行结果...</pre>'
  wrapper.appendChild(outputPanel)

  try {
    const normalizedLang = lang.toLowerCase()

    // HTML 预览 — 使用 iframe（确保 UTF-8 编码）
    if (normalizedLang === 'html') {
      // 如果代码中没有 charset 声明，自动添加 UTF-8 meta 标签
      let htmlCode = code
      if (!code.includes('charset') && !code.includes('CHARSET')) {
        htmlCode = `<meta charset="UTF-8">\n${code}`
      }
      const blob = new Blob([htmlCode], { type: 'text/html;charset=utf-8' })
      const blobUrl = URL.createObjectURL(blob)
      outputPanel.innerHTML = `<div class="output-header"><span>HTML 预览</span><button class="code-block-copy" style="border:none;background:none;cursor:pointer;padding:2px;" onclick="this.closest('.code-execution-output').remove()"><div class="i-carbon-close" style="width:14px;height:14px;display:block;color:#9ca3af;"></div></button></div>`
      const iframe = document.createElement('iframe')
      iframe.className = 'html-preview-frame'
      iframe.sandbox.add('allow-scripts')
      iframe.src = blobUrl
      iframe.style.cssText = 'width:100%;min-height:200px;max-height:500px;border:none;border-top:1px solid #374151;border-radius:0 0 0.5rem 0.5rem;background:#ffffff;'
      outputPanel.appendChild(iframe)
      // 清理 blob URL
      iframe.onload = () => URL.revokeObjectURL(blobUrl)
      return
    }

    // JS — 沙箱 iframe 执行
    if (normalizedLang === 'javascript' || normalizedLang === 'js') {
      const result = await executeJavaScriptInSandbox(code)
      renderExecutionResult(outputPanel, result)
      return
    }

    // 后端语言 — 通过 Tauri invoke 执行
    const result = await invoke<CodeExecutionResult>('execute_code_snippet', {
      request: { language: normalizedLang, code },
    })
    renderExecutionResult(outputPanel, result)
  }
  catch (error) {
    outputPanel.innerHTML = `<div class="output-header"><span class="output-error">执行错误</span></div><pre style="margin:0;padding:0;border:none;background:none;font-size:inherit;color:inherit;" class="output-error">${String(error)}</pre>`
  }
}

// 在沙箱 iframe 中执行 JavaScript
function executeJavaScriptInSandbox(code: string): Promise<CodeExecutionResult> {
  return new Promise((resolve) => {
    const iframe = document.createElement('iframe')
    iframe.style.display = 'none'
    iframe.sandbox.add('allow-scripts')
    document.body.appendChild(iframe)

    const timer = setTimeout(() => {
      window.removeEventListener('message', handler)
      document.body.removeChild(iframe)
      resolve({ stdout: '', stderr: '执行超时（10秒）', exit_code: null, timed_out: true, error: null })
    }, 10000)

    function handler(e: MessageEvent) {
      if (e.source === iframe.contentWindow) {
        clearTimeout(timer)
        window.removeEventListener('message', handler)
        document.body.removeChild(iframe)
        resolve({
          stdout: e.data?.result || '',
          stderr: e.data?.error || '',
          exit_code: e.data?.error ? 1 : 0,
          timed_out: false,
          error: null,
        })
      }
    }
    window.addEventListener('message', handler)

    // 将代码注入 iframe
    const wrappedCode = JSON.stringify(code)
    iframe.srcdoc = `<script>
try {
  const __logs = [];
  console.log = (...args) => __logs.push(args.map(String).join(' '));
  console.warn = (...args) => __logs.push('[WARN] ' + args.map(String).join(' '));
  console.error = (...args) => __logs.push('[ERROR] ' + args.map(String).join(' '));
  const __result = eval(${wrappedCode});
  const __output = __logs.length > 0 ? __logs.join('\\n') : String(__result);
  parent.postMessage({ result: __output }, '*');
} catch(e) {
  parent.postMessage({ error: e.message || String(e) }, '*');
}
<\/script>`
  })
}

// 渲染执行结果
function renderExecutionResult(panel: HTMLElement, result: CodeExecutionResult) {
  const statusClass = result.timed_out ? 'output-timeout' : (result.exit_code === 0 ? 'output-success' : 'output-error')
  const statusText = result.timed_out ? '超时' : (result.exit_code === 0 ? '完成' : `退出码: ${result.exit_code}`)

  let content = ''
  if (result.stdout) content += result.stdout
  if (result.stderr) content += (content ? '\n' : '') + result.stderr
  if (result.error) content += (content ? '\n' : '') + result.error
  if (!content) content = '（无输出）'

  // 转义 HTML
  content = content.replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;')

  panel.innerHTML = `<div class="output-header"><span class="${statusClass}">${statusText}</span><button style="border:none;background:none;cursor:pointer;padding:2px;color:#9ca3af;" onclick="this.closest('.code-execution-output').remove()"><div class="i-carbon-close" style="width:14px;height:14px;display:block;"></div></button></div><pre style="margin:0;padding:0;border:none;background:none;font-size:inherit;color:inherit;">${content}</pre>`
}

interface CodeExecutionResult {
  stdout: string
  stderr: string
  exit_code: number | null
  timed_out: boolean
  error: string | null
}

interface ImagePreviewState {
  src: string
  alt: string
  zoom: number
}

interface Props {
  request: McpRequest | null
  loading?: boolean
  currentTheme?: string
}

interface Emits {
  quoteMessage: [message: string]
}

// 初始化 hljs 主题
onMounted(() => {
  loadHljsTheme('auto', props.currentTheme)
  resizeObserver = new ResizeObserver(() => {
    const root = markdownRoot.value
    if (!root) return
    root.querySelectorAll<HTMLElement>('.mermaid-block-wrapper').forEach((wrapper) => {
      applyMermaidZoom(wrapper, getMermaidZoom(wrapper))
    })
  })
  if (markdownRoot.value) {
    resizeObserver.observe(markdownRoot.value)
  }
  prepareRenderedMarkdown()
})

// 主题变化时重新加载 hljs 样式
watch(() => props.currentTheme, (newTheme) => {
  loadHljsTheme('auto', newTheme)
  prepareRenderedMarkdown()
})

watch(renderedMarkdown, () => {
  prepareRenderedMarkdown()
})

watch(showRawMarkdown, () => {
  prepareRenderedMarkdown()
})

watch(markdownRoot, (newRoot, oldRoot) => {
  if (oldRoot) {
    resizeObserver?.unobserve(oldRoot)
  }
  if (newRoot) {
    resizeObserver?.observe(newRoot)
    prepareRenderedMarkdown()
  }
})

watch(() => props.request?.id, () => {
  showRawMarkdown.value = false
  imagePreview.value = null
})

onBeforeUnmount(() => {
  resizeObserver?.disconnect()
  resizeObserver = null
})
</script>

<template>
  <div class="text-white">
    <!-- 加载状态 -->
    <div v-if="loading" class="flex flex-col items-center justify-center py-8">
      <n-spin size="medium" />
      <p class="text-sm mt-3 text-white opacity-60">
        加载中...
      </p>
    </div>

    <!-- 消息显示区域 -->
    <div v-else-if="request?.message" class="relative">
      <div v-if="request.is_markdown" class="markdown-view-toolbar" :class="markdownThemeClass">
        <div class="markdown-view-tabs" role="tablist" aria-label="Markdown 显示模式">
          <button
            type="button"
            :class="{ active: !showRawMarkdown }"
            title="显示渲染后的 Markdown"
            @click="showRawMarkdown = false"
          >
            <div class="i-carbon-view" />
            <span>预览</span>
          </button>
          <button
            type="button"
            :class="{ active: showRawMarkdown }"
            title="显示原始 Markdown"
            @click="showRawMarkdown = true"
          >
            <div class="i-carbon-code" />
            <span>原始 MD</span>
          </button>
          <!-- 复制原始 Markdown 内容 -->
          <button
            v-if="showRawMarkdown"
            type="button"
            class="markdown-view-copy"
            title="复制原始 Markdown 全部内容"
            @click="copyRawMarkdown"
          >
            <div class="i-carbon-copy" />
            <span>复制</span>
          </button>
        </div>
      </div>

      <!-- Markdown 内容 -->
      <div
        v-if="request.is_markdown && !showRawMarkdown"
        ref="markdownRoot"
        class="markdown-content"
        :class="markdownThemeClass"
        @click="handleMarkdownClick"
        v-html="renderedMarkdown"
      />
      <pre
        v-else-if="request.is_markdown"
        class="markdown-raw-view"
        :class="markdownThemeClass"
      >{{ request.message }}</pre>
      <div v-else class="whitespace-pre-wrap leading-relaxed text-white">
        {{ request.message }}
      </div>

      <!-- 引用原文按钮 -->
      <div class="flex justify-end mt-4 pt-3 border-t border-gray-600/30" data-guide="quote-message">
        <div
          title="点击将AI的消息内容引用到输入框中"
          class="inline-flex items-center gap-1.5 px-3 py-1.5 text-xs font-medium bg-blue-500/20 hover:bg-blue-500/30 text-white rounded-md transition-all duration-200 cursor-pointer border border-blue-500/50 hover:border-blue-500/70 shadow-sm hover:shadow-md"
          @click="quoteMessage"
        >
          <div class="i-carbon-quotes w-3.5 h-3.5" />
          <span>引用原文</span>
        </div>
      </div>
    </div>

    <n-modal
      :show="!!imagePreview"
      preset="card"
      class="markdown-image-preview-modal"
      :bordered="false"
      :mask-closable="true"
      @update:show="(value: boolean) => { if (!value) closeImagePreview() }"
    >
      <template #header>
        <div class="markdown-image-preview-header">
          <span>{{ imagePreview?.alt }}</span>
          <div class="markdown-image-preview-actions">
            <button type="button" title="缩小图片" @click="adjustPreviewZoom(-0.2)">
              <div class="i-carbon-zoom-out" />
            </button>
            <button type="button" title="放大图片" @click="adjustPreviewZoom(0.2)">
              <div class="i-carbon-zoom-in" />
            </button>
            <button type="button" title="重置缩放" @click="resetPreviewZoom">
              <div class="i-carbon-reset" />
            </button>
            <button type="button" title="复制原图" @click="copyPreviewImage">
              <div class="i-carbon-copy" />
            </button>
          </div>
        </div>
      </template>
      <div class="markdown-image-preview-body">
        <img
          v-if="imagePreview"
          :src="imagePreview.src"
          :alt="imagePreview.alt"
          :style="{ transform: `scale(${imagePreview.zoom})` }"
        >
      </div>
    </n-modal>

    <!-- 错误状态 -->
    <n-alert v-if="!loading && !request?.message" type="error" title="数据加载错误">
      <div class="text-white">
        Request对象: {{ JSON.stringify(request) }}
      </div>
    </n-alert>
  </div>
</template>
