<script setup lang="ts">
import type { UiuxConfigData, UiuxEditSession } from '../../types/uiux'
import { invoke } from '@tauri-apps/api/core'
import { useDialog, useMessage } from 'naive-ui'
import { computed, onBeforeUnmount, onMounted, ref, watch } from 'vue'
import ConfigSection from '../common/ConfigSection.vue'

const props = defineProps<{ active: boolean, session?: UiuxEditSession | null }>()
const emit = defineEmits<{ 'update:session': [session: UiuxEditSession] }>()
const message = useMessage()
const dialog = useDialog()

interface UiuxModelStatus {
  phase: 'missing' | 'installed' | 'downloading' | 'verifying' | 'loading' | 'indexing' | 'ready' | 'error'
  embedding_phase: string
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

interface ModelIntegrityResult {
  valid: boolean
  state: string
  model_dir: string
  checked_at: string
  message: string
}

const saved = ref<UiuxConfigData | null>(props.session ? { ...props.session.saved } : null)
const config = ref<UiuxConfigData>(props.session
  ? { ...props.session.draft }
  : {
      knowledge_backend: 'auto',
      semantic_enabled: true,
      model_dir: '',
      effective_model_dir: '',
    })
const status = ref<UiuxModelStatus | null>(null)
const loading = ref(false)
const saving = ref(false)
const operation = ref<'download' | 'verify' | 'remove' | 'cancel' | null>(null)
const configReadError = ref('')
const configConflict = ref(false)
const statusReadError = ref('')
const lastSuccessfulReadAt = ref('')
const statusRequestActive = ref(false)
const integrity = ref<ModelIntegrityResult | null>(null)
let statusTimer: ReturnType<typeof setInterval> | null = null
let statusEpoch = 0
let configEpoch = 0
let pendingStatusRefresh = false
let disposed = false

function editableSignature(value: UiuxConfigData): string {
  return JSON.stringify([value.knowledge_backend, value.semantic_enabled, (value.model_dir || '').trim()])
}

function sameConfig(left: UiuxConfigData, right: UiuxConfigData): boolean {
  return editableSignature(left) === editableSignature(right)
    && left.effective_model_dir === right.effective_model_dir
}

const dirty = computed(() => !!saved.value && editableSignature(config.value) !== editableSignature(saved.value))
const directoryDirty = computed(() => !!saved.value
  && (config.value.model_dir || '').trim() !== (saved.value.model_dir || '').trim())
const formDisabled = computed(() => loading.value || saving.value || !saved.value)

// 中文说明：同步发布两份独立快照，工具组件卸载后由父级保留草稿，绝不缓存请求和计时器。
watch([saved, config], () => {
  if (saved.value && !disposed)
    emit('update:session', { saved: { ...saved.value }, draft: { ...config.value } })
}, { deep: true, flush: 'sync' })

const backendOptions = [
  { label: '自动：本地 BM25 + BGE', value: 'auto' },
  { label: '仅本地 BM25', value: 'local' },
  { label: 'Fast Context（A/B 诊断）', value: 'fast_context' },
]

const phaseLabel = computed(() => {
  const labels: Record<string, string> = {
    missing: '未下载',
    installed: '已安装，按需加载',
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
const embeddingPhaseLabel = computed(() => {
  const labels: Record<string, string> = { loading: '加载中', ready: '已加载', error: '加载异常' }
  const phase = status.value?.embedding_phase || ''
  if (labels[phase])
    return labels[phase]
  if (status.value?.phase === 'installed' || status.value?.embedding_phase === 'unloaded')
    return '尚未加载或已闲置释放，下次查询按需加载'
  return phase === 'missing' ? '等待模型资产' : '待读取'
})

const taskActive = computed(() =>
  ['downloading', 'verifying', 'loading', 'indexing'].includes(status.value?.phase || ''),
)
const modelActionsDisabled = computed(() => formDisabled.value || !!operation.value || taskActive.value
  || directoryDirty.value || configConflict.value || !!configReadError.value || !!statusReadError.value
  || !status.value || status.value.model_dir !== saved.value?.effective_model_dir)
const saveDisabled = computed(() => formDisabled.value || !!operation.value || taskActive.value
  || !dirty.value || configConflict.value || !!configReadError.value)

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

function invalidateStatus(clear = false) {
  statusEpoch++
  stopPolling()
  if (clear) {
    status.value = null
    statusReadError.value = ''
    lastSuccessfulReadAt.value = ''
    integrity.value = null
  }
}

function acceptConfig(value: UiuxConfigData) {
  if (saved.value?.effective_model_dir !== value.effective_model_dir)
    invalidateStatus(true)
  saved.value = { ...value }
  config.value = { ...value }
  configConflict.value = false
}

function restoreDraft() {
  if (saved.value && !formDisabled.value && !operation.value)
    config.value = { ...saved.value }
}

async function refreshStatus(showError = false) {
  if (disposed || !props.active || !saved.value)
    return
  if (statusRequestActive.value) {
    pendingStatusRefresh = true
    return
  }
  const epoch = statusEpoch
  const directory = saved.value.effective_model_dir
  statusRequestActive.value = true
  try {
    const result = await invoke<UiuxModelStatus>('get_uiux_model_status')
    if (disposed || !props.active || epoch !== statusEpoch)
      return
    if (result.model_dir !== directory) {
      configConflict.value = true
      statusReadError.value = '生效目录已变化，请加载当前配置后重试'
      return
    }
    status.value = result
    statusReadError.value = ''
    lastSuccessfulReadAt.value = new Date().toLocaleString()
    if (taskActive.value)
      startPolling()
    else
      stopPolling()
  }
  catch (error) {
    if (disposed || !props.active || epoch !== statusEpoch)
      return
    statusReadError.value = String(error)
    if (showError)
      message.error(`读取模型状态失败: ${error}`)
  }
  finally {
    statusRequestActive.value = false
    // 中文说明：旧目录请求真正结束后才补发刷新，代次失效不应产生第二个并行状态请求。
    if (pendingStatusRefresh && !disposed && props.active) {
      pendingStatusRefresh = false
      void refreshStatus()
    }
  }
}

async function loadAll(discardDraft = false) {
  const epoch = ++configEpoch
  loading.value = true
  try {
    const result = await invoke<UiuxConfigData>('get_uiux_config')
    if (disposed || !props.active || epoch !== configEpoch)
      return
    configReadError.value = ''
    if (discardDraft || !saved.value || !dirty.value || editableSignature(result) === editableSignature(config.value)) {
      acceptConfig(result)
    }
    else {
      configConflict.value = !sameConfig(result, saved.value)
    }
    // 中文说明：模型状态独立刷新，慢状态请求不阻塞已经读取成功的配置编辑。
    void refreshStatus()
  }
  catch (error) {
    if (!disposed && props.active && epoch === configEpoch)
      configReadError.value = String(error)
  }
  finally {
    if (epoch === configEpoch)
      loading.value = false
  }
}

async function saveConfig(showFeedback = true): Promise<boolean> {
  if (disposed || !props.active || saveDisabled.value || !saved.value)
    return false
  saving.value = true
  try {
    const result = await invoke<UiuxConfigData>('set_uiux_config', {
      config: {
        ...config.value,
        model_dir: (config.value.model_dir || '').trim() || null,
      },
      expectedConfig: { ...saved.value },
    })
    if (disposed)
      return true
    acceptConfig(result)
    if (showFeedback)
      message.success('UIUX 配置已保存')
    // 中文说明：写入成功由后端快照确认，后续状态读取失败单独展示，不误报保存失败。
    void refreshStatus()
    return true
  }
  catch (error) {
    if (!disposed) {
      message.error(`保存 UIUX 配置失败: ${error}`)
      // 中文说明：冲突检查仍保留草稿；重新读取基线可识别其他入口刚刚保存的配置。
      void loadAll()
    }
    return false
  }
  finally {
    saving.value = false
  }
}

async function selectDirectory() {
  if (formDisabled.value || operation.value || taskActive.value)
    return
  try {
    const selected = await invoke<string | null>('select_uiux_model_directory', {
      defaultPath: config.value.model_dir || config.value.effective_model_dir,
    })
    if (selected && !disposed)
      config.value.model_dir = selected
  }
  catch (error) {
    message.error(`选择模型目录失败: ${error}`)
  }
}

async function startDownload() {
  if (disposed || !props.active || modelActionsDisabled.value || !saved.value)
    return
  const expectedModelDir = saved.value.effective_model_dir
  operation.value = 'download'
  invalidateStatus()
  const epoch = statusEpoch
  try {
    const result = await invoke<UiuxModelStatus>('start_uiux_model_download', { expectedModelDir })
    if (disposed || !props.active || epoch !== statusEpoch || result.model_dir !== expectedModelDir)
      return
    status.value = result
    void refreshStatus()
    message.success('模型下载任务已启动')
  }
  catch (error) {
    if (!disposed && props.active)
      message.error(`启动模型下载失败: ${error}`)
  }
  finally {
    operation.value = null
  }
}

async function verifyModel() {
  if (disposed || !props.active || modelActionsDisabled.value || !saved.value)
    return
  const expectedModelDir = saved.value.effective_model_dir
  const epoch = statusEpoch
  operation.value = 'verify'
  try {
    const result = await invoke<ModelIntegrityResult>('verify_uiux_model_integrity', { expectedModelDir })
    if (!disposed && props.active && epoch === statusEpoch && result.model_dir === expectedModelDir)
      integrity.value = result
  }
  catch (error) {
    if (!disposed && props.active)
      message.error(`模型校验失败: ${error}`)
  }
  finally {
    operation.value = null
  }
}

async function cancelDownload() {
  if (disposed || !props.active || operation.value || status.value?.phase !== 'downloading')
    return
  operation.value = 'cancel'
  try {
    await invoke('cancel_uiux_model_download')
    message.info('已请求取消模型下载')
    await refreshStatus()
  }
  catch (error) {
    message.error(`取消模型下载失败: ${error}`)
  }
  finally {
    operation.value = null
  }
}

function confirmRemove() {
  if (disposed || !props.active || modelActionsDisabled.value || !saved.value)
    return
  const expectedModelDir = saved.value.effective_model_dir
  dialog.warning({
    title: '删除本地语义模型',
    content: `目标目录：${expectedModelDir}。将删除供 UIUX 与 Sou 共享的 BGE 模型、未完成分片和语义索引缓存；共享 ONNX Runtime 保留。`,
    positiveText: '删除',
    negativeText: '取消',
    onPositiveClick: async () => {
      if (disposed || !props.active || modelActionsDisabled.value || saved.value?.effective_model_dir !== expectedModelDir)
        return false
      operation.value = 'remove'
      invalidateStatus()
      const epoch = statusEpoch
      try {
        const result = await invoke<UiuxModelStatus>('remove_uiux_model', { expectedModelDir })
        if (disposed || !props.active || epoch !== statusEpoch || result.model_dir !== expectedModelDir)
          return
        status.value = result
        integrity.value = null
        void refreshStatus()
        message.success('本地语义模型已删除')
      }
      catch (error) {
        if (!disposed && props.active)
          message.error(`删除模型失败: ${error}`)
      }
      finally {
        operation.value = null
      }
    },
  })
}

function startPolling() {
  if (statusTimer || !props.active || disposed)
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
    configEpoch++
    invalidateStatus()
    pendingStatusRefresh = false
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
onBeforeUnmount(() => {
  disposed = true
  configEpoch++
  invalidateStatus()
  pendingStatusRefresh = false
})

defineExpose({ saveConfig })
</script>

<template>
  <div class="uiux-config">
    <n-scrollbar class="config-scrollbar">
      <n-spin :show="loading">
        <n-space vertical size="large" class="config-content">
          <n-alert v-if="configReadError" type="error" :bordered="false">
            读取配置失败：{{ configReadError }}。已保留输入。
            <n-button text @click="loadAll()">
              重试读取
            </n-button>
          </n-alert>
          <n-alert v-if="configConflict" type="warning" :bordered="false">
            配置已在其他入口更新，当前草稿已保留。加载当前配置会撤销本页未保存输入。
            <n-button text :disabled="saving || !!operation" @click="loadAll(true)">
              加载当前配置
            </n-button>
          </n-alert>
          <ConfigSection title="检索策略" description="auto 在本机融合 BM25 与 BGE；显式 fast_context 仅用于对比诊断">
            <n-space vertical size="medium">
              <n-form-item label="默认知识后端">
                <n-select v-model:value="config.knowledge_backend" :options="backendOptions" :disabled="formDisabled" />
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
                <n-switch v-model:value="config.semantic_enabled" :disabled="formDisabled" />
              </div>
              <n-alert v-if="dirty" type="info" :bordered="false">
                有未保存更改；模型操作仅使用已生效目录，且不会自动保存检索策略。
              </n-alert>
              <div class="actions end">
                <n-button secondary :disabled="formDisabled || !dirty || !!operation || configConflict" @click="restoreDraft">
                  恢复已保存配置
                </n-button>
                <n-button type="primary" :loading="saving" :disabled="saveDisabled" @click="saveConfig()">
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
                    :placeholder="saved?.effective_model_dir || '自动选择目录'"
                    :disabled="formDisabled || taskActive || !!operation"
                  />
                  <n-tooltip trigger="hover">
                    <template #trigger>
                      <n-button :disabled="formDisabled || taskActive || !!operation" aria-label="选择模型目录" @click="selectDirectory">
                        <template #icon>
                          <div class="i-carbon-folder" />
                        </template>
                      </n-button>
                    </template>
                    选择模型目录
                  </n-tooltip>
                </n-input-group>
                <template #feedback>
                  <span class="path-feedback">已生效目录：{{ saved?.effective_model_dir || '待读取' }}</span>
                </template>
              </n-form-item>
              <n-alert v-if="directoryDirty" type="warning" :bordered="false">
                待保存目录：{{ config.model_dir || '自动选择目录（保存后显示实际路径）' }}。请先保存或恢复配置，再下载、校验或删除模型。
              </n-alert>

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
                    ORT 资产 {{ status?.runtime_ready ? '齐备' : '未齐备' }}
                  </n-tag>
                  <n-tag :type="phaseType" :bordered="false">
                    {{ phaseLabel }}
                  </n-tag>
                </div>
              </div>
              <div v-if="status?.embedding_phase" class="control-help">
                推理状态：{{ embeddingPhaseLabel }}
              </div>
              <div class="actions">
                <span class="control-help">最近成功读取：{{ lastSuccessfulReadAt || '尚未读取' }}</span>
                <n-button quaternary circle :loading="statusRequestActive" :disabled="!saved || loading" aria-label="刷新模型状态" @click="refreshStatus(true)">
                  <template #icon>
                    <div class="i-carbon-renew" />
                  </template>
                </n-button>
              </div>
              <n-alert v-if="statusReadError" type="warning" :bordered="false">
                状态更新失败：{{ statusReadError }}。{{ status ? '以下为上次成功读取的结果。' : '请重试读取状态。' }}
              </n-alert>

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
              <n-alert v-if="integrity" :type="integrity.valid ? 'success' : 'error'" :bordered="false">
                {{ integrity.message }}
                <div class="path-feedback">
                  校验目录：{{ integrity.model_dir }} · {{ integrity.checked_at }}
                </div>
              </n-alert>

              <div class="actions">
                <n-button
                  v-if="status?.phase === 'downloading'"
                  secondary
                  type="warning"
                  :loading="operation === 'cancel'"
                  :disabled="!!operation"
                  @click="cancelDownload"
                >
                  <template #icon>
                    <div class="i-carbon-stop-filled" />
                  </template>
                  取消下载
                </n-button>
                <n-button
                  v-else-if="!['installed', 'ready'].includes(status?.phase || '') || integrity?.valid === false"
                  type="primary"
                  :loading="operation === 'download'"
                  :disabled="modelActionsDisabled"
                  @click="startDownload"
                >
                  <template #icon>
                    <div class="i-carbon-download" />
                  </template>
                  {{ integrity?.valid === false ? '修复下载' : status?.phase === 'error' ? '重试下载' : '下载模型' }}
                </n-button>
                <n-button secondary :loading="operation === 'verify'" :disabled="modelActionsDisabled" @click="verifyModel">
                  <template #icon>
                    <div class="i-carbon-security" />
                  </template>
                  重新校验
                </n-button>
                <n-tooltip trigger="hover">
                  <template #trigger>
                    <n-button
                      tertiary
                      type="error"
                      :disabled="modelActionsDisabled || !(status?.model_downloaded_bytes || status?.indexed_documents || 0)"
                      :loading="operation === 'remove'"
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
