const MAX_DIMENSION = 1920
const JPEG_QUALITY_LARGE = 0.85
const JPEG_QUALITY_SMALL = 0.92
const LARGE_THRESHOLD = 2000

export interface CompressionResult {
  dataUrl: string
  originalBytes: number
  compressedBytes: number
  format: 'jpeg' | 'png'
  width: number
  height: number
  wasResized: boolean
}

function dataUrlToBytes(dataUrl: string): number {
  const base64 = dataUrl.split(',')[1]
  if (!base64) return 0
  let padding = 0
  if (base64.endsWith('==')) padding = 2
  else if (base64.endsWith('=')) padding = 1
  return Math.floor((base64.length * 3) / 4) - padding
}

function loadImage(dataUrl: string): Promise<HTMLImageElement> {
  return new Promise((resolve, reject) => {
    const img = new Image()
    img.onload = () => resolve(img)
    img.onerror = reject
    img.src = dataUrl
  })
}

function hasTransparency(canvas: HTMLCanvasElement): boolean {
  const w = Math.min(canvas.width, 100)
  const h = Math.min(canvas.height, 100)

  const tempCanvas = document.createElement('canvas')
  tempCanvas.width = w
  tempCanvas.height = h
  const tempCtx = tempCanvas.getContext('2d')
  if (!tempCtx) return false
  tempCtx.drawImage(canvas, 0, 0, w, h)

  const data = tempCtx.getImageData(0, 0, w, h).data
  for (let i = 3; i < data.length; i += 4) {
    if (data[i] < 255) return true
  }
  return false
}

function parseMimeFromDataUrl(dataUrl: string): string {
  const match = dataUrl.match(/^data:(image\/[^;]+);/)
  return match ? match[1] : 'image/unknown'
}

export async function compressImage(dataUrl: string): Promise<CompressionResult> {
  const originalBytes = dataUrlToBytes(dataUrl)
  const img = await loadImage(dataUrl)

  let { width, height } = img
  let wasResized = false

  const maxSide = Math.max(width, height)
  if (maxSide > MAX_DIMENSION) {
    const ratio = MAX_DIMENSION / maxSide
    width = Math.round(width * ratio)
    height = Math.round(height * ratio)
    wasResized = true
  }

  const canvas = document.createElement('canvas')
  canvas.width = width
  canvas.height = height
  const ctx = canvas.getContext('2d')
  if (!ctx) {
    return {
      dataUrl,
      originalBytes,
      compressedBytes: originalBytes,
      format: parseMimeFromDataUrl(dataUrl).indexOf('png') !== -1 ? 'png' : 'jpeg',
      width: img.width,
      height: img.height,
      wasResized: false,
    }
  }
  ctx.drawImage(img, 0, 0, width, height)

  const transparent = hasTransparency(canvas)

  let format: 'jpeg' | 'png'
  let compressedDataUrl: string

  if (transparent) {
    format = 'png'
    compressedDataUrl = canvas.toDataURL('image/png')
  } else {
    format = 'jpeg'
    const quality = maxSide > LARGE_THRESHOLD ? JPEG_QUALITY_LARGE : JPEG_QUALITY_SMALL
    compressedDataUrl = canvas.toDataURL('image/jpeg', quality)
  }

  const compressedBytes = dataUrlToBytes(compressedDataUrl)

  if (compressedBytes >= originalBytes) {
    return {
      dataUrl,
      originalBytes,
      compressedBytes: originalBytes,
      format: parseMimeFromDataUrl(dataUrl).indexOf('png') !== -1 ? 'png' : 'jpeg',
      width: img.width,
      height: img.height,
      wasResized: false,
    }
  }

  return {
    dataUrl: compressedDataUrl,
    originalBytes,
    compressedBytes,
    format,
    width,
    height,
    wasResized,
  }
}

export function formatBytes(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`
  return `${(bytes / (1024 * 1024)).toFixed(1)} MB`
}

export function compressionSummary(result: CompressionResult): string {
  const ratio = result.originalBytes > 0
    ? Math.round((1 - result.compressedBytes / result.originalBytes) * 100)
    : 0

  if (ratio <= 0 && !result.wasResized) return ''

  const parts = [`${formatBytes(result.originalBytes)} → ${formatBytes(result.compressedBytes)}`]
  if (ratio > 0) parts.push(`-${ratio}%`)
  if (result.wasResized) parts.push(`缩放至 ${result.width}×${result.height}`)
  parts.push(result.format.toUpperCase())
  return parts.join(' | ')
}
