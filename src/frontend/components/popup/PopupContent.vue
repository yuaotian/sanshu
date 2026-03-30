<script setup lang="ts">
import hljs from 'highlight.js'
import MarkdownIt from 'markdown-it'
import { useMessage } from 'naive-ui'
import { open } from '@tauri-apps/plugin-shell'
import { nextTick, onMounted, onUpdated, watch } from 'vue'
import type { McpRequest } from '../../types/popup'
import { sanitizeHtml } from '../../utils/sanitize'

const props = withDefaults(defineProps<Props>(), {
  loading: false,
  currentTheme: 'dark',
})

const emit = defineEmits<Emits>()

// 预处理引用内容，移除增强prompt格式标记
function preprocessQuoteContent(content: string): string {
  let processedContent = content

  // 定义需要移除的格式标记
  const markersToRemove = [
    /### BEGIN RESPONSE ###\s*/gi,
    /Here is an enhanced version of the original instruction that is more specific and clear:\s*/gi,
    /<augment-enhanced-prompt>\s*/gi,
    /<\/augment-enhanced-prompt>\s*/gi,
    /### END RESPONSE ###\s*/gi,
  ]

  // 逐个移除格式标记
  markersToRemove.forEach((marker) => {
    processedContent = processedContent.replace(marker, '')
  })

  // 清理多余的空行和首尾空白
  processedContent = processedContent
    .replace(/\n\s*\n\s*\n/g, '\n\n') // 将多个连续空行合并为两个
    .trim() // 移除首尾空白

  return processedContent
}

// 引用消息内容
function quoteMessage() {
  if (props.request?.message) {
    // 预处理内容，移除增强prompt格式标记
    const processedContent = preprocessQuoteContent(props.request.message)
    emit('quoteMessage', processedContent)
  }
}

// 动态导入代码高亮样式，根据主题切换

// 动态加载代码高亮样式
function loadHighlightStyle(theme: string) {
  // 移除现有的highlight.js样式
  const existingStyle = document.querySelector('link[data-highlight-theme]')
  if (existingStyle) {
    existingStyle.remove()
  }

  // 根据主题选择样式
  const styleName = theme === 'light' ? 'github' : 'github-dark'

  // 动态创建样式链接
  const link = document.createElement('link')
  link.rel = 'stylesheet'
  link.href = `https://cdnjs.cloudflare.com/ajax/libs/highlight.js/11.9.0/styles/${styleName}.min.css`
  link.setAttribute('data-highlight-theme', theme)
  document.head.appendChild(link)
}

interface Props {
  request: McpRequest | null
  loading?: boolean
  currentTheme?: string
}

interface Emits {
  quoteMessage: [message: string]
}

const message = useMessage()

// 创建 Markdown 实例 - 保持代码高亮功能
const md = new MarkdownIt({
  html: true,
  xhtmlOut: false,
  breaks: true,
  langPrefix: 'language-',
  linkify: true,
  typographer: true,
  quotes: '""\'\'',
  highlight(str: string, lang: string) {
    if (lang && hljs.getLanguage(lang)) {
      try {
        return hljs.highlight(str, { language: lang }).value
      }
      catch {
        // 忽略错误
      }
    }
    return ''
  },
})

// 自定义链接渲染器 - 外部链接用系统浏览器打开
md.renderer.rules.link_open = function (tokens, idx, options, env, renderer) {
  const token = tokens[idx]
  const href = token.attrGet('href')

  if (href && (href.startsWith('http://') || href.startsWith('https://'))) {
    token.attrSet('href', '#')
    token.attrSet('data-external-url', href)
    token.attrSet('title', href)
  }

  return renderer.renderToken(tokens, idx, options)
}

md.renderer.rules.autolink_open = function (tokens, idx, options, env, renderer) {
  const token = tokens[idx]
  const href = token.attrGet('href')

  if (href && (href.startsWith('http://') || href.startsWith('https://'))) {
    token.attrSet('href', '#')
    token.attrSet('data-external-url', href)
    token.attrSet('title', href)
  }

  return renderer.renderToken(tokens, idx, options)
}

function handleExternalLinkClick(e: Event) {
  const target = (e.target as HTMLElement).closest('a[data-external-url]')
  if (!target) return
  e.preventDefault()
  const url = target.getAttribute('data-external-url')
  if (url) open(url)
}

function renderMarkdown(content: string) {
  try {
    return sanitizeHtml(md.render(content))
  }
  catch (error) {
    console.error('Markdown 渲染失败:', error)
    return sanitizeHtml(content)
  }
}

// 创建复制按钮（放在 pre 外层 wrapper 上，避免随横向滚动移动）
function createCopyButton(preEl: Element) {
  const parent = preEl.parentElement
  if (!parent)
    return

  // 如果已经包裹过，跳过
  if (parent.classList.contains('code-block-wrapper'))
    return

  const wrapper = document.createElement('div')
  wrapper.className = 'code-block-wrapper'
  wrapper.style.cssText = 'position: relative;'
  parent.insertBefore(wrapper, preEl)
  wrapper.appendChild(preEl)

  const copyButton = document.createElement('div')
  copyButton.className = 'copy-button'
  copyButton.style.cssText = `
    position: absolute;
    top: 8px;
    right: 8px;
    z-index: 10;
    height: 20px;
    width: 20px;
    display: flex;
    align-items: center;
    justify-content: center;
    pointer-events: auto;
    opacity: 0;
    transition: opacity 0.15s ease;
  `

  copyButton.innerHTML = `
    <button style="
      display: flex;
      align-items: center;
      justify-content: center;
      width: 100%;
      height: 100%;
      color: var(--color-on-surface-muted);
      transition: color 0.2s ease;
      border: none;
      background: none;
      cursor: pointer;
      padding: 0;
      margin: 0;
    " onmouseover="this.style.color='var(--color-primary)'" onmouseout="this.style.color='var(--color-on-surface-muted)'">
      <div class="i-carbon-copy" style="width: 16px; height: 16px; display: block;"></div>
    </button>
  `

  const button = copyButton.querySelector('button')!
  button.addEventListener('click', async (e) => {
    e.stopPropagation()
    e.preventDefault()
    try {
      const codeEl = preEl.querySelector('code')
      const textContent = codeEl?.textContent || preEl.textContent || ''
      await navigator.clipboard.writeText(textContent)

      const icon = button.querySelector('div')!
      icon.className = 'i-carbon-checkmark'
      icon.style.cssText = `width: 16px; height: 16px; color: var(--color-success); display: block;`

      setTimeout(() => {
        icon.className = 'i-carbon-copy'
        icon.style.cssText = 'width: 16px; height: 16px; display: block;'
      }, 2000)
      message.success('代码已复制到剪贴板')
    }
    catch {
      message.error('复制失败')
    }
  })

  wrapper.appendChild(copyButton)
}

// 设置内联代码复制
function setupInlineCodeCopy() {
  const inlineCodeElements = document.querySelectorAll('.markdown-content p code, .markdown-content li code')
  inlineCodeElements.forEach((codeEl) => {
    codeEl.addEventListener('click', async () => {
      try {
        await navigator.clipboard.writeText(codeEl.textContent || '')
        message.success('代码已复制到剪贴板')
      }
      catch {
        message.error('复制失败')
      }
    })
  })
}

// 设置代码复制功能
let setupCodeCopyTimer: number | null = null
function setupCodeCopy() {
  if (setupCodeCopyTimer) {
    clearTimeout(setupCodeCopyTimer)
  }

  // 增加延迟时间，确保DOM完全渲染
  setupCodeCopyTimer = window.setTimeout(() => {
    nextTick(() => {
      // 确保选择正确的 pre 元素
      const preElements = document.querySelectorAll('.markdown-content pre')
      console.log('设置代码复制按钮，找到', preElements.length, '个代码块')
      preElements.forEach((preEl) => {
        createCopyButton(preEl)
      })
      setupInlineCodeCopy()

      // 如果没有找到代码块，再次尝试
      if (preElements.length === 0) {
        setTimeout(() => {
          const retryElements = document.querySelectorAll('.markdown-content pre')
          console.log('重试设置代码复制按钮，找到', retryElements.length, '个代码块')
          retryElements.forEach((preEl) => {
            createCopyButton(preEl)
          })
        }, 200)
      }
    })
  }, 300)
}

// 监听request变化，重新设置代码复制
watch(() => props.request, () => {
  if (props.request) {
    setupCodeCopy()
  }
}, { deep: true })

// 监听loading状态变化
watch(() => props.loading, (newLoading) => {
  if (!newLoading && props.request) {
    setupCodeCopy()
  }
})

onMounted(() => {
  // 初始化代码高亮样式
  loadHighlightStyle(props.currentTheme)
  if (props.request) {
    setupCodeCopy()
  }
})

// 监听主题变化
watch(() => props.currentTheme, (newTheme) => {
  loadHighlightStyle(newTheme)
}, { immediate: false })

// 在DOM更新后也尝试设置
onUpdated(() => {
  if (props.request && !props.loading) {
    setupCodeCopy()
  }
})
</script>

<template>
  <div class="text-on-surface">
    <!-- 加载状态 -->
    <div v-if="loading" class="flex flex-col items-center justify-center py-8">
      <n-spin size="medium" />
      <p class="text-sm mt-3 text-on-surface-muted">
        加载中...
      </p>
    </div>

    <!-- 消息显示区域 -->
    <div v-else-if="request?.message" class="relative">
      <!-- 主要内容 -->
      <div
        v-if="request.is_markdown"
        class="markdown-content prose prose-sm max-w-none prose-headings:font-semibold prose-headings:leading-tight prose-h1:!mt-3 prose-h1:!mb-1.5 prose-h1:!text-base prose-h1:!font-semibold prose-h2:!mt-2.5 prose-h2:!mb-1 prose-h2:!text-sm prose-h2:!font-semibold prose-h3:!mt-2 prose-h3:!mb-1 prose-h3:!text-sm prose-h3:!font-medium prose-h4:!mt-1.5 prose-h4:!mb-0.5 prose-h4:!text-xs prose-h4:!font-medium prose-p:my-1 prose-p:leading-relaxed prose-p:text-sm prose-ul:my-1 prose-ul:text-sm prose-ul:pl-4 prose-ol:my-1 prose-ol:text-sm prose-ol:pl-4 prose-li:my-0.5 prose-li:text-sm prose-li:leading-relaxed prose-blockquote:my-1.5 prose-blockquote:text-sm prose-blockquote:pl-3 prose-blockquote:ml-0 prose-blockquote:italic prose-blockquote:border-l-2 prose-blockquote:border-primary-500 prose-pre:border prose-pre:rounded-[3px] prose-pre:p-3 prose-pre:my-2 prose-pre:overflow-x-auto prose-pre:text-xs scrollbar-code prose-code:px-1 prose-code:py-0.5 prose-code:text-xs prose-code:rounded-[2px] prose-code:cursor-pointer prose-code:font-mono prose-a:text-primary-500 prose-a:no-underline prose-a:cursor-pointer" :class="[
          currentTheme === 'light' ? 'prose-slate' : 'prose-invert',
          'prose-headings:text-on-surface',
          currentTheme === 'light' ? 'prose-p:text-on-surface-secondary' : 'prose-p:text-on-surface prose-p:opacity-85',
          currentTheme === 'light' ? 'prose-ul:text-on-surface-secondary prose-ol:text-on-surface-secondary prose-li:text-on-surface-secondary' : 'prose-ul:text-on-surface prose-ul:opacity-85 prose-ol:text-on-surface prose-ol:opacity-85 prose-li:text-on-surface prose-li:opacity-85',
          currentTheme === 'light' ? 'prose-blockquote:text-on-surface-muted' : 'prose-blockquote:text-on-surface-secondary prose-blockquote:opacity-90',
          'prose-pre:bg-container-secondary prose-pre:border-border',
          'prose-strong:text-on-surface prose-strong:font-semibold',
          currentTheme === 'light' ? 'prose-em:text-on-surface-muted prose-em:italic' : 'prose-em:text-on-surface-secondary prose-em:italic',
        ]"
        v-html="renderMarkdown(request.message)"
        @click="handleExternalLinkClick"
      />
      <div v-else class="whitespace-pre-wrap leading-relaxed text-on-surface">
        {{ request.message }}
      </div>

      <!-- 引用原文按钮 -->
      <div class="flex justify-end mt-4 pt-3 border-t border-border/30" data-guide="quote-message">
        <n-button
          size="small"
          secondary
          type="info"
          title="点击将AI的消息内容引用到输入框中"
          @click="quoteMessage"
        >
          <template #icon>
            <div class="i-carbon-quotes w-3.5 h-3.5" />
          </template>
          引用原文
        </n-button>
      </div>
    </div>

    <!-- 错误状态 -->
    <n-alert v-else type="error" title="数据加载错误">
      <div class="text-on-surface">
        Request对象: {{ JSON.stringify(request) }}
      </div>
    </n-alert>
  </div>
</template>

<style scoped>
:deep(.markdown-content pre) {
  max-width: 100%;
  overflow-x: auto;
}

:deep(.markdown-content code:not(pre code)) {
  background: var(--color-container-secondary);
  color: var(--color-on-surface);
}

:deep(.markdown-content a[data-external-url]) {
  text-decoration: underline;
  text-underline-offset: 2px;
}

:deep(.markdown-content a[data-external-url]:hover) {
  opacity: 0.8;
}

:deep(.markdown-content hr) {
  margin: 0.5rem 0;
  border-color: var(--color-border);
  opacity: 0.5;
}

:deep(.markdown-content table) {
  font-size: 12px;
  margin: 0.5rem 0;
  border-collapse: collapse;
  width: 100%;
}

:deep(.markdown-content th),
:deep(.markdown-content td) {
  padding: 6px 10px;
  border: 1px solid var(--color-border);
  text-align: left;
}

:deep(.markdown-content th) {
  background: var(--color-container-secondary);
  font-weight: 600;
}

:deep(.markdown-content tr:hover td) {
  background: color-mix(in srgb, var(--color-on-surface) 3%, transparent);
}

:deep(.code-block-wrapper:hover .copy-button) {
  opacity: 1 !important;
}

:deep(.code-block-wrapper pre) {
  padding-top: 0.75rem !important;
}
</style>
