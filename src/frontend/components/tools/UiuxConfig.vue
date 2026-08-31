<script setup lang="ts">
import { invoke } from '@tauri-apps/api/core'
import { useDialog, useMessage } from 'naive-ui'
import { computed, onBeforeUnmount, onMounted, ref, watch } from 'vue'
import ConfigSection from '../common/ConfigSection.vue'

const props = defineProps<{ active: boolean }>()
const message = useMessage()
const dialog = useDialog()

interface UiuxConfigData {
  knowledge_backend: 'auto' | 'fast_context' | 'local'
  semantic_enabled: boolean
  model_dir: string
  effective_model_dir: string
}

interface UiuxModelStatus {
  phase: 'missing' | 'downloading' | 'verifying' | 'loading' | 'indexing' | 'ready' | 'error'
  model_name: string
  revision: string
  model_dir: string
  downloaded_bytes: number
  total_bytes: number
  model_downloaded_bytes: number
  model_total_bytes: number
  completed_files: number
  total_files: number
  runtime_version: string
  runtime_ready: boolean
  runtime_dir: string
  runtime_downloaded_bytes: number
  runtime_total_bytes: number
  indexed_documents: number
  total_documents: number
  progress_percent: number
  index_progress_percent: number
  route?: string
  message: string
  error?: string
  updated_at: string
}

const config = ref<UiuxConfigData>({
  knowledge_backend: 'auto',
  semantic_enabled: true,
  model_dir: '',
  effective_model_dir: '',
})
const status = ref<UiuxModelStatus | null>(null)
const loading = ref(false)
const saving = ref(false)
const operating = ref(false)
let statusTimer: ReturnType<typeof setInterval> | null = null

const backendOptions = [
  { label: '自动：本地 BM25 + BGE', value: 'auto' },
  { label: '仅本地 BM25', value: 'local' },
  { label: 'Fast Context（A/B 诊断）', value: 'fast_context' },
]

const phaseLabel = computed(() => {
  const labels: Record<string, string> = {
    missing: '未下载',
    downloading: '下载中',
    verifying: '校验中',
    loading: '加载中',
    indexing: '建索引中',
    ready: '已就绪',
    error: '异常',
  }
  return labels[status.value?.phase || 'missing'] || status.value?.phase || '未知'
})

const phaseType = computed<'success' | 'error' | 'warning' | 'info' | 'default'>(() => {
  if (status.value?.phase === 'ready')
    return 'success'
  if (status.value?.phase === 'error')
    return 'error'
  if (status.value?.phase === 'missing')
    return 'warning'
  if (['downloading', 'verifying', 'loading', 'indexing'].includes(status.value?.phase || ''))
    return 'info'
  return 'default'
})

const taskActive = computed(() =>
  ['downloading', 'verifying', 'loading', 'indexing'].includes(status.value?.phase || ''),
)

const visibleProgress = computed(() =>
  status.value?.phase === 'indexing'
    ? status.value.index_progress_percent
    : status.value?.progress_percent || 0,
)

function formatBytes(value: number): string {
  if (!Number.isFinite(value) || value <= 0)
    return '0 B'
  const units = ['B', 'KB', 'MB', 'GB']
  const index = Math.min(Math.floor(Math.log(value) / Math.log(1024)), units.length - 1)
  return `${(value / 1024 ** index).toFixed(index === 0 ? 0 : 1)} ${units[index]}`
}

async function loadConfig() {
  const result = await invoke<{
    knowledge_backend: UiuxConfigData['knowledge_backend']
    semantic_enabled: boolean
    model_dir?: string
    effective_model_dir: string
  }>('get_uiux_config')
  config.value = {
    ...result,
    model_dir: result.model_dir || '',
  }
}

async function refreshStatus(showError = false) {
  try {
    status.value = await invoke<UiuxModelStatus>('get_uiux_model_status')
  }
  catch (error) {
    if (showError)
      message.error(`读取模型状态失败: ${error}`)
  }
}

async function loadAll() {
  loading.value = true
  try {
    await Promise.all([loadConfig(), refreshStatus(true)])
  }
  catch (error) {
    message.error(`加载 UIUX 配置失败: ${error}`)
  }
  finally {
    loading.value = false
  }
}

async function saveConfig(showFeedback = true): Promise<boolean> {
  saving.value = true
  try {
    await invoke('set_uiux_config', {
      config: {
        ...config.value,
        model_dir: config.value.model_dir.trim() || null,
      },
    })
    await loadConfig()
    await refreshStatus()
    if (showFeedback)
      message.success('UIUX 配置已保存')
    return true
  }
  catch (error) {
    message.error(`保存 UIUX 配置失败: ${error}`)
    return false
  }
  finally {
    saving.value = false
  }
}

async function selectDirectory() {
  try {
    const selected = await invoke<string | null>('select_uiux_model_directory', {
      defaultPath: config.value.model_dir || config.value.effective_model_dir,
    })
    if (selected)
      config.value.model_dir = selected
  }
  catch (error) {
    message.error(`选择模型目录失败: ${error}`)
  }
}

async function startDownload() {
  operating.value = true
  try {
    if (!await saveConfig(false))
      return
    status.value = await invoke<UiuxModelStatus>('start_uiux_model_download')
    startPolling()
    message.success('模型下载任务已启动')
  }
  catch (error) {
    message.error(`启动模型下载失败: ${error}`)
  }
  finally {
    operating.value = false
  }
}

async function cancelDownload() {
  try {
    await invoke('cancel_uiux_model_download')
    message.info('已请求取消模型下载')
    await refreshStatus()
  }
  catch (error) {
    message.error(`取消模型下载失败: ${error}`)
  }
}

function confirmRemove() {
  dialog.warning({
    title: '删除本地语义模型',
    content: '将删除 BGE 模型、未完成分片和语义索引缓存；共享 ONNX Runtime 会保留供后续复用。',
    positiveText: '删除',
    negativeText: '取消',
    onPositiveClick: async () => {
      operating.value = true
      try {
        status.value = await invoke<UiuxModelStatus>('remove_uiux_model')
        message.success('本地语义模型已删除')
      }
      catch (error) {
        message.error(`删除模型失败: ${error}`)
      }
      finally {
        operating.value = false
      }
    },
  })
}

function startPolling() {
  if (statusTimer || !props.active)
    return
  statusTimer = setInterval(async () => {
    await refreshStatus()
    if (!taskActive.value)
      stopPolling()
  }, 1000)
}

function stopPolling() {
  if (statusTimer) {
    clearInterval(statusTimer)
    statusTimer = null
  }
}

watch(() => props.active, async (active) => {
  if (active) {
    await loadAll()
    if (taskActive.value)
      startPolling()
  }
  else {
    stopPolling()
  }
})

onMounted(async () => {
  if (props.active) {
    await loadAll()
    if (taskActive.value)
      startPolling()
  }
})
onBeforeUnmount(stopPolling)

defineExpose({ saveConfig })
</script>

<template>
  <div class="uiux-config">
    <n-scrollbar class="config-scrollbar">
      <n-spin :show="loading">
        <n-space vertical size="large" class="config-content">
          <ConfigSection title="检索策略" description="auto 在本机融合 BM25 与 BGE；显式 fast_context 仅用于对比诊断">
            <n-space vertical size="medium">
              <n-form-item label="默认知识后端">
                <n-select v-model:value="config.knowledge_backend" :options="backendOptions" />
              </n-form-item>
              <div class="switch-row">
                <div>
                  <div class="control-title">
                    启用 BGE 语义增强
                  </div>
                  <div class="control-help">
                    模型未就绪或 2 秒内未完成加载时使用 BM25
                  </div>
                </div>
                <n-switch v-model:value="config.semantic_enabled" />
              </div>
              <div class="actions end">
                <n-button type="primary" :loading="saving" @click="saveConfig()">
                  <template #icon>
                    <div class="i-carbon-save" />
                  </template>
                  保存配置
                </n-button>
              </div>
            </n-space>
          </ConfigSection>

          <ConfigSection title="本地模型" description="Xenova/bge-small-zh-v1.5 · 512 维 · 固定版本与 SHA-256 校验">
            <n-space vertical size="medium">
              <n-form-item label="模型目录">
                <n-input-group>
                  <n-input
                    v-model:value="config.model_dir"
                    :placeholder="config.effective_model_dir"
                    :disabled="taskActive"
                  />
                  <n-tooltip trigger="hover">
                    <template #trigger>
                      <n-button :disabled="taskActive" aria-label="选择模型目录" @click="selectDirectory">
                        <template #icon>
                          <div class="i-carbon-folder" />
                        </template>
                      </n-button>
                    </template>
                    选择模型目录
                  </n-tooltip>
                </n-input-group>
                <template #feedback>
                  <span class="path-feedback">{{ config.model_dir || config.effective_model_dir }}</span>
                </template>
              </n-form-item>

              <div class="status-header">
                <div>
                  <div class="control-title">
                    {{ status?.model_name || 'Xenova/bge-small-zh-v1.5' }}
                  </div>
                  <div class="control-help">
                    {{ status?.message || '正在读取模型状态' }}
                  </div>
                </div>
                <div class="status-tags">
                  <n-tag :type="status?.runtime_ready ? 'success' : 'warning'" :bordered="false">
                    ORT {{ status?.runtime_version || '1.28.0' }}
                  </n-tag>
                  <n-tag :type="phaseType" :bordered="false">
                    {{ phaseLabel }}
                  </n-tag>
                </div>
              </div>

              <n-progress
                type="line"
                :percentage="Math.max(0, Math.min(100, visibleProgress))"
                :status="status?.phase === 'error' ? 'error' : status?.phase === 'ready' ? 'success' : 'default'"
                :height="8"
                :border-radius="4"
              />

              <div class="metrics">
                <span>总计 {{ formatBytes(status?.downloaded_bytes || 0) }} / {{ formatBytes(status?.total_bytes || 0) }}</span>
                <span>BGE {{ formatBytes(status?.model_downloaded_bytes || 0) }} / {{ formatBytes(status?.model_total_bytes || 0) }}</span>
                <span>ORT {{ formatBytes(status?.runtime_downloaded_bytes || 0) }} / {{ formatBytes(status?.runtime_total_bytes || 0) }}</span>
                <span>文件 {{ status?.completed_files || 0 }} / {{ status?.total_files || 0 }}</span>
                <span>索引 {{ status?.indexed_documents || 0 }} / {{ status?.total_documents || 0 }}</span>
              </div>
              <div v-if="status?.route" class="path-feedback">
                当前路由：{{ status.route }}
              </div>
              <div v-if="status?.runtime_dir" class="path-feedback">
                运行时目录：{{ status.runtime_dir }}
              </div>

              <n-alert v-if="status?.error" type="error" :bordered="false">
                {{ status.error }}
              </n-alert>

              <div class="actions">
                <n-button
                  v-if="status?.phase === 'downloading'"
                  secondary
                  type="warning"
                  @click="cancelDownload"
                >
                  <template #icon>
                    <div class="i-carbon-stop-filled" />
                  </template>
                  取消下载
                </n-button>
                <n-button
                  v-else
                  type="primary"
                  :loading="operating"
                  :disabled="taskActive"
                  @click="startDownload"
                >
                  <template #icon>
                    <div class="i-carbon-download" />
                  </template>
                  {{ status?.phase === 'ready' ? '重新校验' : '下载模型' }}
                </n-button>
                <n-tooltip trigger="hover">
                  <template #trigger>
                    <n-button
                      tertiary
                      type="error"
                      :disabled="taskActive || !(status?.model_downloaded_bytes || status?.indexed_documents || 0)"
                      aria-label="删除本地模型"
                      @click="confirmRemove"
                    >
                      <template #icon>
                        <div class="i-carbon-trash-can" />
                      </template>
                    </n-button>
                  </template>
                  删除本地模型
                </n-tooltip>
              </div>
            </n-space>
          </ConfigSection>
        </n-space>
      </n-spin>
    </n-scrollbar>
  </div>
</template>

<style scoped>
.uiux-config {
  height: 100%;
}

.config-scrollbar {
  max-height: 65vh;
}

.config-content {
  padding-right: 8px;
  padding-bottom: 16px;
}

.switch-row,
.status-header,
.status-tags,
.actions,
.metrics {
  display: flex;
  align-items: center;
}

.switch-row,
.status-header {
  justify-content: space-between;
  gap: 16px;
}

.status-tags {
  gap: 6px;
  flex-shrink: 0;
}

.control-title {
  color: var(--color-on-surface, #111827);
  font-size: 13px;
  font-weight: 600;
}

.control-help,
.path-feedback,
.metrics {
  color: var(--color-on-surface-secondary, #6b7280);
  font-size: 11px;
}

.path-feedback {
  overflow-wrap: anywhere;
}

.metrics {
  justify-content: space-between;
  gap: 12px;
  flex-wrap: wrap;
}

.actions {
  gap: 8px;
}

.actions.end {
  justify-content: flex-end;
}

:root.dark .control-title {
  color: #e5e7eb;
}

:root.dark .control-help,
:root.dark .path-feedback,
:root.dark .metrics {
  color: #9ca3af;
}

@media (max-width: 640px) {
  .switch-row,
  .status-header {
    align-items: flex-start;
  }

  .metrics {
    flex-direction: column;
    align-items: flex-start;
    gap: 4px;
  }
}
</style>
