import DOMPurify from 'dompurify'

const htmlPurify = DOMPurify(window)

htmlPurify.setConfig({
  ALLOWED_TAGS: [
    'h1', 'h2', 'h3', 'h4', 'h5', 'h6',
    'p', 'br', 'hr', 'blockquote', 'pre', 'code',
    'ul', 'ol', 'li', 'dl', 'dt', 'dd',
    'table', 'thead', 'tbody', 'tr', 'th', 'td',
    'strong', 'em', 'del', 's', 'mark', 'sub', 'sup',
    'a', 'img', 'span', 'div',
  ],
  ALLOWED_ATTR: [
    'href', 'target', 'rel', 'src', 'alt', 'title',
    'class', 'id', 'width', 'height',
    'align', 'valign', 'colspan', 'rowspan',
    'data-external-url', 'data-highlight-theme',
  ],
  ALLOW_DATA_ATTR: false,
  ADD_ATTR: ['target'],
})

const svgPurify = DOMPurify(window)

svgPurify.setConfig({
  USE_PROFILES: { svg: true, svgFilters: true },
  ADD_TAGS: ['use'],
  FORBID_TAGS: ['script', 'foreignObject'],
  FORBID_ATTR: [
    'onload', 'onerror', 'onclick', 'onmouseover',
    'onfocus', 'onblur', 'onanimationend',
  ],
})

export function sanitizeHtml(dirty: string): string {
  return htmlPurify.sanitize(dirty) as string
}

export function sanitizeSvg(dirty: string): string {
  return svgPurify.sanitize(dirty) as string
}
