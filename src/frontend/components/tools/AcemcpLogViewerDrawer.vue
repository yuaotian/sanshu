<script setup lang="ts">
import { invoke } from '@tauri-apps/api/core'
import { listen } from '@tauri-apps/api/event'
import { useMessage } from 'naive-ui'
import { computed, nextTick, onBeforeUnmount, ref, watch } from 'vue'
import { useVirtualizer } from '@tanstack/vue-virtual'

interface Props {
  show: boolean
}

interface Emits {
  'update:show': [value: boolean]
}

type LogStreamEventType = 'append' | 'error'

interface AcemcpLogStreamEvent {
  event_type: LogStreamEventType
  lines: string[]
  error?: string
}

interface ParsedLogLine {
  raw: string
  timestamp: string
  level: string
  module: string
  message: string
}

const props = defineProps<Props>()
const emit = defineEmits<Emits>()

const message = useMessage()

const maxLines = ref(5000)
const autoScroll = ref(true)
const keyword = ref('')
const selectedLevels = ref<string[]>(['ERROR', 'WARN', 'INFO', 'DEBUG', 'TRACE'])

const isLoading = ref(false)
const isStreaming = ref(false)

const allItems = ref<ParsedLogLine[]>([])
let unlistenStream: (() => void) | null = null

const levelOptions = [
  { label: 'ERROR', value: 'ERROR' },
  { label: 'WARN', value: 'WARN' },
  { label: 'INFO', value: 'INFO' },
  { label: 'DEBUG', value: 'DEBUG' },
  { label: 'TRACE', value: 'TRACE' },
]

const maxLineOptions = [
  { label: '1000 行', value: 1000 },
  { label: '2000 行', value: 2000 },
  { label: '3000 行', value: 3000 },
  { label: '5000 行', value: 5000 },
]

const LOG_RE = /^(\d{4}-\d{2}-\d{2} \d{2}:\d{2}:\d{2}\.\d{3}) \[([A-Z]+)\] \[([^\]]+)\] (.*)$/

function parseLine(raw: string): ParsedLogLine {
  const m = LOG_RE.exec(raw)
  if (!m) {
    return {
      raw,
      timestamp: '',
      level: 'INFO',
      module: '',
      message: raw,
    }
  }

  const [, ts, level, modulePath, msg] = m
  return {
    raw,
    timestamp: ts,
    level,
    module: modulePath,
    message: msg,
  }
}

function clampBuffer() {
  const overflow = allItems.value.length - maxLines.value
  if (overflow > 0) {
    allItems.value.splice(0, overflow)
  }
}

function levelClass(level: string) {
  switch ((level || '').toUpperCase()) {
    case 'ERROR':
      return 'bg-red-500/20 text-red-300'
    case 'WARN':
      return 'bg-amber-500/20 text-amber-300'
    case 'INFO':
      return 'bg-slate-500/10 text-slate-200'
    case 'DEBUG':
      return 'bg-slate-500/10 text-slate-400'
    case 'TRACE':
      return 'bg-slate-500/10 text-slate-500'
    default:
      return 'bg-slate-500/10 text-slate-300'
  }
}

function splitHighlight(text: string, q: string): { text: string, hit: boolean }[] {
  const query = (q || '').trim()
  if (!query)
    return [{ text, hit: false }]

  const lowerText = text.toLowerCase()
  const lowerQuery = query.toLowerCase()
  const out: { text: string, hit: boolean }[] = []

  let start = 0
  while (start < text.length) {
    const idx = lowerText.indexOf(lowerQuery, start)
    if (idx < 0) {
      out.push({ text: text.slice(start), hit: false })
      break
    }
    if (idx > start)
      out.push({ text: text.slice(start, idx), hit: false })
    out.push({ text: text.slice(idx, idx + query.length), hit: true })
    start = idx + query.length
  }

  return out.length > 0 ? out : [{ text, hit: false }]
}

const filteredItems = computed(() => {
  const levels = new Set(selectedLevels.value.map(l => l.toUpperCase()))
  const q = keyword.value.trim().toLowerCase()

  if (!q && levels.size === 0)
    return allItems.value

  return allItems.value.filter((it) => {
    const lv = (it.level || '').toUpperCase()
    if (levels.size > 0 && !levels.has(lv))
      return false
    if (!q)
      return true
    return it.raw.toLowerCase().includes(q)
  })
})

const parentRef = ref<HTMLElement | null>(null)
const rowVirtualizer = useVirtualizer({
  count: () => filteredItems.value.length,
  getScrollElement: () => parentRef.value,
  estimateSize: () => 24,
  overscan: 24,
})

const virtualRows = computed(() => rowVirtualizer.value.getVirtualItems())
const totalSize = computed(() => rowVirtualizer.value.getTotalSize())

async function scrollToBottom() {
  if (!autoScroll.value)
    return
  await nextTick()
  const idx = filteredItems.value.length - 1
  if (idx >= 0)
    rowVirtualizer.value.scrollToIndex(idx, { align: 'end' })
}

function appendLines(lines: string[]) {
  if (!Array.isArray(lines) || lines.length === 0)
    return

  for (const raw of lines) {
    if (typeof raw !== 'string' || !raw.trim())
      continue
    allItems.value.push(parseLine(raw))
  }
  clampBuffer()
}

async function loadInitial() {
  isLoading.value = true
  try {
    const lines = await invoke('read_acemcp_logs', { maxLines: maxLines.value }) as string[]
    allItems.value = (lines || []).map(parseLine)
    clampBuffer()
  }
  catch (e: any) {
    message.error(`加载日志失败: ${e?.message || String(e)}`)
  }
  finally {
    isLoading.value = false
  }
}

async function startStream() {
  if (isStreaming.value)
    return

  try {
    unlistenStream = await listen<AcemcpLogStreamEvent>('acemcp-log-stream', async (event) => {
      const payload = event.payload
      if (!payload)
        return

      if (payload.event_type === 'error') {
        if (payload.error)
          message.error(payload.error)
        return
      }

      if (payload.event_type === 'append') {
        appendLines(payload.lines || [])
        await scrollToBottom()
      }
    })

    await invoke('start_acemcp_log_stream', { intervalMs: 250 })
    isStreaming.value = true
  }
  catch (e: any) {
    message.error(`启动实时日志失败: ${e?.message || String(e)}`)
    if (unlistenStream) {
      unlistenStream()
      unlistenStream = null
    }
    isStreaming.value = false
  }
}

async function stopStream() {
  try {
    await invoke('stop_acemcp_log_stream')
  }
  catch {}

  if (unlistenStream) {
    unlistenStream()
    unlistenStream = null
  }
  isStreaming.value = false
}

async function reloadAll() {
  await loadInitial()
  await scrollToBottom()
}

async function clearDisplay() {
  allItems.value = []
}

async function copyVisible() {
  try {
    const text = filteredItems.value.map(i => i.raw).join('\n')
    await navigator.clipboard.writeText(text)
    message.success(`已复制 ${filteredItems.value.length} 行`)
  }
  catch (e: any) {
    message.error(`复制失败: ${e?.message || String(e)}`)
  }
}

watch(() => props.show, async (show) => {
  if (show) {
    await reloadAll()
    await startStream()
    await scrollToBottom()
  }
  else {
    await stopStream()
  }
}, { immediate: true })

watch(maxLines, async () => {
  clampBuffer()
  await scrollToBottom()
})

onBeforeUnmount(async () => {
  await stopStream()
})
</script>

<template>
  <n-drawer
    :show="props.show"
    placement="right"
    width="92%"
    @update:show="emit('update:show', $event)"
  >
    <n-drawer-content title="acemcp 实时日志" closable>
      <div class="flex flex-col gap-3">
        <div class="flex flex-wrap items-center gap-2">
          <n-input
            v-model:value="keyword"
            size="small"
            clearable
            placeholder="关键词搜索（匹配整行）"
            class="w-64"
          />

          <n-select
            v-model:value="selectedLevels"
            size="small"
            multiple
            :options="levelOptions"
            class="w-56"
            placeholder="级别过滤"
          />

          <div class="flex items-center gap-2">
            <span class="text-xs opacity-70">自动滚动</span>
            <n-switch v-model:value="autoScroll" size="small" />
          </div>

          <n-select
            v-model:value="maxLines"
            size="small"
            :options="maxLineOptions"
            class="w-28"
          />

          <n-button size="small" secondary :loading="isLoading" @click="reloadAll">
            刷新
          </n-button>
          <n-button size="small" secondary @click="copyVisible">
            复制可见
          </n-button>
          <n-button size="small" secondary @click="clearDisplay">
            清空显示
          </n-button>

          <n-tag v-if="isStreaming" type="success" size="small">
            实时中
          </n-tag>
          <n-tag v-else type="warning" size="small">
            已暂停
          </n-tag>
        </div>

        <div class="text-xs opacity-60">
          显示 {{ filteredItems.length }} / {{ allItems.length }} 行（缓冲上限 {{ maxLines }}）
        </div>

        <div
          ref="parentRef"
          class="h-[70vh] overflow-auto rounded-md border border-slate-700/30 bg-black/10"
        >
          <div class="relative w-full" :style="{ height: `${totalSize}px` }">
            <div
              v-for="v in virtualRows"
              :key="v.key"
              class="absolute left-0 w-full"
              :style="{ transform: `translateY(${v.start}px)` }"
            >
              <div class="h-6 px-2 flex items-center gap-2 border-b border-slate-700/20 text-xs font-mono">
                <span class="w-52 shrink-0 text-slate-400">
                  {{ filteredItems[v.index]?.timestamp }}
                </span>
                <span class="w-14 shrink-0 text-center rounded px-1" :class="levelClass(filteredItems[v.index]?.level)">
                  {{ filteredItems[v.index]?.level }}
                </span>
                <span class="w-80 shrink-0 truncate text-slate-300" :title="filteredItems[v.index]?.module">
                  {{ filteredItems[v.index]?.module }}
                </span>
                <span class="flex-1 truncate text-slate-100" :title="filteredItems[v.index]?.raw">
                  <template v-if="keyword.trim()">
                    <span
                      v-for="(seg, i) in splitHighlight(filteredItems[v.index]?.message || filteredItems[v.index]?.raw || '', keyword)"
                      :key="i"
                      :class="seg.hit ? 'bg-yellow-400/20 text-yellow-200' : ''"
                    >{{ seg.text }}</span>
                  </template>
                  <template v-else>
                    {{ filteredItems[v.index]?.message || filteredItems[v.index]?.raw }}
                  </template>
                </span>
              </div>
            </div>
          </div>
        </div>
      </div>
    </n-drawer-content>
  </n-drawer>
</template>

