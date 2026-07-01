<script setup lang="ts">
/**
 * 记忆历史清理面板
 * 提供预览、逐条确认、自动备份、恢复和导出能力。
 */
import { invoke } from '@tauri-apps/api/core'
import { useMessage } from 'naive-ui'
import { computed, onMounted, ref, watch } from 'vue'
import ConfigSection from '../common/ConfigSection.vue'

const props = defineProps<{
  projectPath: string
  upsertThreshold: number
}>()

const emit = defineEmits<{
  (e: 'changed'): void
}>()

interface CleanupEntry {
  id: string
  content: string
  category: string
  created_at: string
  updated_at: string
}

interface CleanupGroup {
  group_id: string
  category: string
  max_similarity: number
  recommended_keep_id: string
  default_delete_ids: string[]
  entries: CleanupEntry[]
}

interface CleanupPreviewResult {
  original_count: number
  candidate_group_count: number
  estimated_removed_count: number
  groups: CleanupGroup[]
}

interface CleanupApplyGroup {
  group_id: string
  keep_id: string
  delete_ids: string[]
}

interface CleanupApplyResult {
  backup_file: string | null
  removed_count: number
  remaining_count: number
  removed_ids: string[]
}

interface BackupInfo {
  file_name: string
  created_at: string
  size_bytes: number
  entry_count: number
}

interface RestoreBackupResult {
  restored_file: string
  safety_backup_file: string | null
  entry_count: number
}

const message = useMessage()

const categoryOptions = ['规范', '偏好', '模式', '背景']
const cleanupThresholdPercent = ref(55)
const selectedCategories = ref<string[]>([...categoryOptions])
const includeCrossCategory = ref(false)

const previewLoading = ref(false)
const applyLoading = ref(false)
const backupsLoading = ref(false)
const restoringFile = ref<string | null>(null)
const exportingFile = ref<string | null>(null)

const previewResult = ref<CleanupPreviewResult | null>(null)
const lastApplyResult = ref<CleanupApplyResult | null>(null)
const backups = ref<BackupInfo[]>([])
const keepSelections = ref<Record<string, string>>({})
const deleteSelections = ref<Record<string, string[]>>({})

const selectedRemovalCount = computed(() =>
  buildApplyGroups().reduce((total, group) => total + group.delete_ids.length, 0),
)

watch(
  () => props.upsertThreshold,
  (threshold) => {
    cleanupThresholdPercent.value = Math.round((threshold || 0.55) * 100)
  },
  { immediate: true },
)

watch(
  () => props.projectPath,
  async (projectPath) => {
    if (projectPath)
      await loadBackups()
  },
)

onMounted(async () => {
  if (props.projectPath)
    await loadBackups()
})

async function previewCleanup() {
  if (!props.projectPath)
    return
  if (selectedCategories.value.length === 0) {
    message.warning('请至少选择一个分类')
    return
  }

  previewLoading.value = true
  try {
    const result = await invoke<CleanupPreviewResult>('preview_memory_cleanup', {
      projectPath: props.projectPath,
      request: {
        threshold: cleanupThresholdPercent.value / 100,
        categories: selectedCategories.value,
        include_cross_category: includeCrossCategory.value,
      },
    })
    previewResult.value = result
    lastApplyResult.value = null
    resetSelections(result.groups)
    if (result.candidate_group_count === 0)
      message.info('未发现可清理的历史重复记忆')
    else
      message.success(`发现 ${result.candidate_group_count} 个候选组，预计可移除 ${result.estimated_removed_count} 条`)
  }
  catch (err) {
    message.error(`预览整理失败: ${err}`)
  }
  finally {
    previewLoading.value = false
  }
}

async function applyCleanup() {
  if (!props.projectPath)
    return
  const groups = buildApplyGroups()
  if (groups.length === 0) {
    message.info('没有选择需要删除的记忆')
    return
  }

  applyLoading.value = true
  try {
    const result = await invoke<CleanupApplyResult>('apply_memory_cleanup', {
      projectPath: props.projectPath,
      request: {
        auto_backup: true,
        groups,
      },
    })
    lastApplyResult.value = result
    previewResult.value = null
    resetSelections([])
    message.success(`整理完成：移除 ${result.removed_count} 条，已自动备份`)
    await loadBackups()
    emit('changed')
  }
  catch (err) {
    message.error(`应用整理失败: ${err}`)
  }
  finally {
    applyLoading.value = false
  }
}

async function loadBackups() {
  if (!props.projectPath)
    return
  backupsLoading.value = true
  try {
    backups.value = await invoke<BackupInfo[]>('list_memory_backups', {
      projectPath: props.projectPath,
    })
  }
  catch (err) {
    message.error(`读取备份失败: ${err}`)
  }
  finally {
    backupsLoading.value = false
  }
}

async function restoreBackup(fileName: string) {
  if (!props.projectPath)
    return
  restoringFile.value = fileName
  try {
    const result = await invoke<RestoreBackupResult>('restore_memory_backup', {
      projectPath: props.projectPath,
      fileName,
    })
    message.success(`已恢复备份：${result.restored_file}`)
    previewResult.value = null
    lastApplyResult.value = null
    await loadBackups()
    emit('changed')
  }
  catch (err) {
    message.error(`恢复备份失败: ${err}`)
  }
  finally {
    restoringFile.value = null
  }
}

async function exportBackup(fileName: string) {
  if (!props.projectPath)
    return
  exportingFile.value = fileName
  try {
    const content = await invoke<string>('export_memory_backup', {
      projectPath: props.projectPath,
      fileName,
    })
    downloadText(fileName, content)
    message.success('备份已导出')
  }
  catch (err) {
    message.error(`导出备份失败: ${err}`)
  }
  finally {
    exportingFile.value = null
  }
}

function resetSelections(groups: CleanupGroup[]) {
  const keep: Record<string, string> = {}
  const deletes: Record<string, string[]> = {}
  for (const group of groups) {
    keep[group.group_id] = group.recommended_keep_id
    deletes[group.group_id] = [...group.default_delete_ids]
  }
  keepSelections.value = keep
  deleteSelections.value = deletes
}

function setKeep(group: CleanupGroup, keepId: string) {
  keepSelections.value = {
    ...keepSelections.value,
    [group.group_id]: keepId,
  }
  deleteSelections.value = {
    ...deleteSelections.value,
    [group.group_id]: group.entries
      .filter(entry => entry.id !== keepId)
      .map(entry => entry.id),
  }
}

function setDelete(group: CleanupGroup, entryId: string, checked: boolean) {
  const keepId = keepSelections.value[group.group_id]
  if (entryId === keepId)
    return

  const current = new Set(deleteSelections.value[group.group_id] || [])
  if (checked)
    current.add(entryId)
  else
    current.delete(entryId)

  deleteSelections.value = {
    ...deleteSelections.value,
    [group.group_id]: Array.from(current),
  }
}

function isDeleteSelected(groupId: string, entryId: string): boolean {
  return (deleteSelections.value[groupId] || []).includes(entryId)
}

function buildApplyGroups(): CleanupApplyGroup[] {
  const groups = previewResult.value?.groups || []
  return groups
    .map((group) => {
      const keepId = keepSelections.value[group.group_id] || group.recommended_keep_id
      const deleteIds = (deleteSelections.value[group.group_id] || [])
        .filter(id => id !== keepId)
      return {
        group_id: group.group_id,
        keep_id: keepId,
        delete_ids: deleteIds,
      }
    })
    .filter(group => group.delete_ids.length > 0)
}

function formatPercent(value: number): string {
  return `${(value * 100).toFixed(1)}%`
}

function formatDate(value: string): string {
  try {
    return new Date(value).toLocaleString('zh-CN')
  }
  catch {
    return value
  }
}

function formatBytes(bytes: number): string {
  if (bytes < 1024)
    return `${bytes} B`
  if (bytes < 1024 * 1024)
    return `${(bytes / 1024).toFixed(1)} KB`
  return `${(bytes / 1024 / 1024).toFixed(1)} MB`
}

function downloadText(fileName: string, content: string) {
  const blob = new Blob([content], { type: 'application/json;charset=utf-8' })
  const url = URL.createObjectURL(blob)
  const link = document.createElement('a')
  link.href = url
  link.download = fileName
  link.click()
  URL.revokeObjectURL(url)
}
</script>

<template>
  <div class="memory-cleanup-panel">
    <ConfigSection title="历史清理预览" description="先预览候选组，再逐条确认保留和删除">
      <n-space vertical size="medium">
        <div class="cleanup-controls">
          <n-form-item label="清理阈值">
            <div class="w-full">
              <div class="flex items-center gap-4">
                <n-slider
                  v-model:value="cleanupThresholdPercent"
                  :min="40"
                  :max="90"
                  :step="5"
                  :marks="{ 40: '40%', 55: '55%', 90: '90%' }"
                  class="flex-1"
                />
                <n-tag type="warning" :bordered="false">
                  {{ cleanupThresholdPercent }}%
                </n-tag>
              </div>
              <div class="help-text">
                默认使用同类更新阈值，低于去重阈值时可发现历史近义堆积。
              </div>
            </div>
          </n-form-item>

          <n-form-item label="参与分类">
            <n-checkbox-group v-model:value="selectedCategories">
              <n-space>
                <n-checkbox v-for="category in categoryOptions" :key="category" :value="category">
                  {{ category }}
                </n-checkbox>
              </n-space>
            </n-checkbox-group>
          </n-form-item>

          <div class="switch-item compact">
            <div class="switch-info">
              <div class="switch-label">
                允许跨分类候选
              </div>
              <div class="switch-desc">
                默认关闭，避免规范、偏好、背景之间误合并
              </div>
            </div>
            <n-switch v-model:value="includeCrossCategory" />
          </div>
        </div>

        <n-space>
          <n-button type="primary" :loading="previewLoading" @click="previewCleanup">
            <template #icon>
              <div class="i-carbon-search" />
            </template>
            预览整理
          </n-button>
          <n-button secondary :loading="backupsLoading" @click="loadBackups">
            <template #icon>
              <div class="i-carbon-renew" />
            </template>
            刷新备份
          </n-button>
        </n-space>
      </n-space>
    </ConfigSection>

    <ConfigSection v-if="previewResult" title="候选分组" :no-card="true">
      <n-alert v-if="previewResult.groups.length === 0" type="success" :bordered="false">
        未发现可清理的历史重复记忆
      </n-alert>

      <n-space v-else vertical size="medium">
        <div class="cleanup-summary">
          <n-tag type="info" :bordered="false">
            候选组 {{ previewResult.candidate_group_count }}
          </n-tag>
          <n-tag type="warning" :bordered="false">
            预计移除 {{ selectedRemovalCount }} / {{ previewResult.estimated_removed_count }}
          </n-tag>
          <n-popconfirm
            :disabled="selectedRemovalCount === 0"
            @positive-click="applyCleanup"
          >
            <template #trigger>
              <n-button
                type="error"
                secondary
                :disabled="selectedRemovalCount === 0"
                :loading="applyLoading"
              >
                <template #icon>
                  <div class="i-carbon-clean" />
                </template>
                应用整理
              </n-button>
            </template>
            将自动备份当前 memories.json，并删除已选的 {{ selectedRemovalCount }} 条记忆。是否继续？
          </n-popconfirm>
        </div>

        <div v-for="group in previewResult.groups" :key="group.group_id" class="cleanup-group">
          <div class="group-header">
            <div class="group-title">
              <div class="i-carbon-compare text-lg text-teal-600 dark:text-teal-300" />
              <span>{{ group.category }} · {{ group.group_id }}</span>
            </div>
            <n-tag type="warning" :bordered="false">
              最高相似 {{ formatPercent(group.max_similarity) }}
            </n-tag>
          </div>

          <n-radio-group
            :value="keepSelections[group.group_id]"
            @update:value="value => setKeep(group, String(value))"
          >
            <div class="entry-grid">
              <div
                v-for="entry in group.entries"
                :key="entry.id"
                class="cleanup-entry"
                :class="{ selected: keepSelections[group.group_id] === entry.id }"
              >
                <div class="entry-toolbar">
                  <n-radio :value="entry.id">
                    保留
                  </n-radio>
                  <n-checkbox
                    :checked="isDeleteSelected(group.group_id, entry.id)"
                    :disabled="keepSelections[group.group_id] === entry.id"
                    @update:checked="checked => setDelete(group, entry.id, Boolean(checked))"
                  >
                    删除
                  </n-checkbox>
                </div>
                <div class="entry-content">
                  {{ entry.content }}
                </div>
                <div class="entry-meta">
                  <span>{{ entry.id }}</span>
                  <span>{{ formatDate(entry.updated_at) }}</span>
                </div>
              </div>
            </div>
          </n-radio-group>
        </div>
      </n-space>
    </ConfigSection>

    <ConfigSection v-if="lastApplyResult" title="上次清理结果" :no-card="true">
      <n-alert type="success" :bordered="false">
        移除 <strong>{{ lastApplyResult.removed_count }}</strong> 条记忆，
        保留 <strong>{{ lastApplyResult.remaining_count }}</strong> 条。
        <span v-if="lastApplyResult.backup_file">备份：{{ lastApplyResult.backup_file }}</span>
      </n-alert>
    </ConfigSection>

    <ConfigSection title="自动备份" description="最多保留最近 10 个备份，恢复前会再次备份当前状态">
      <div v-if="backupsLoading" class="backup-list">
        <n-skeleton v-for="i in 3" :key="i" text :repeat="2" />
      </div>
      <div v-else-if="backups.length === 0" class="empty-backups">
        <div class="i-carbon-archive text-4xl mb-2 opacity-20" />
        <div class="text-sm opacity-60">
          暂无自动备份
        </div>
      </div>
      <div v-else class="backup-list">
        <div v-for="backup in backups" :key="backup.file_name" class="backup-item">
          <div class="backup-main">
            <div class="backup-name">
              {{ backup.file_name }}
            </div>
            <div class="backup-meta">
              {{ formatDate(backup.created_at) }} · {{ backup.entry_count }} 条 · {{ formatBytes(backup.size_bytes) }}
            </div>
          </div>
          <n-space size="small">
            <n-button
              text
              type="primary"
              :loading="exportingFile === backup.file_name"
              @click="exportBackup(backup.file_name)"
            >
              <template #icon>
                <div class="i-carbon-download" />
              </template>
              导出
            </n-button>
            <n-popconfirm @positive-click="restoreBackup(backup.file_name)">
              <template #trigger>
                <n-button
                  text
                  type="warning"
                  :loading="restoringFile === backup.file_name"
                >
                  <template #icon>
                    <div class="i-carbon-undo" />
                  </template>
                  恢复
                </n-button>
              </template>
              恢复前会自动备份当前状态。确认恢复此备份？
            </n-popconfirm>
          </n-space>
        </div>
      </div>
    </ConfigSection>
  </div>
</template>

<style scoped>
.memory-cleanup-panel {
  display: flex;
  flex-direction: column;
  gap: 16px;
}

.cleanup-controls {
  display: flex;
  flex-direction: column;
  gap: 12px;
}

.help-text,
.switch-desc,
.entry-meta,
.backup-meta {
  color: var(--color-on-surface-secondary, #6b7280);
  font-size: 12px;
}

:root.dark .help-text,
:root.dark .switch-desc,
:root.dark .entry-meta,
:root.dark .backup-meta {
  color: #d1d5db;
}

.switch-item {
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: 12px 16px;
  border: 1px solid var(--color-border, rgba(128, 128, 128, 0.15));
  border-radius: 8px;
  background: var(--color-container, rgba(255, 255, 255, 0.5));
}

.switch-item.compact {
  min-height: 56px;
}

:root.dark .switch-item {
  border-color: rgba(255, 255, 255, 0.08);
  background: rgba(24, 24, 28, 0.5);
}

.switch-label {
  color: var(--color-on-surface, #111827);
  font-size: 14px;
  font-weight: 500;
}

:root.dark .switch-label {
  color: #f3f4f6;
}

.cleanup-summary {
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  gap: 8px;
}

.cleanup-group {
  padding: 12px;
  border: 1px solid var(--color-border, rgba(128, 128, 128, 0.15));
  border-radius: 8px;
  background: var(--color-container, rgba(255, 255, 255, 0.5));
}

:root.dark .cleanup-group {
  border-color: rgba(255, 255, 255, 0.08);
  background: rgba(24, 24, 28, 0.5);
}

.group-header {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
  margin-bottom: 12px;
}

.group-title {
  display: flex;
  align-items: center;
  gap: 8px;
  min-width: 0;
  color: var(--color-on-surface, #111827);
  font-size: 14px;
  font-weight: 600;
}

:root.dark .group-title {
  color: #f3f4f6;
}

.entry-grid {
  display: grid;
  grid-template-columns: repeat(auto-fit, minmax(220px, 1fr));
  gap: 10px;
}

.cleanup-entry {
  display: flex;
  flex-direction: column;
  gap: 8px;
  min-height: 160px;
  padding: 10px;
  border: 1px solid rgba(128, 128, 128, 0.14);
  border-radius: 8px;
  background: rgba(255, 255, 255, 0.45);
}

.cleanup-entry.selected {
  border-color: rgba(20, 184, 166, 0.5);
  background: rgba(20, 184, 166, 0.08);
}

:root.dark .cleanup-entry {
  border-color: rgba(255, 255, 255, 0.08);
  background: rgba(17, 24, 39, 0.4);
}

.entry-toolbar {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 8px;
}

.entry-content {
  flex: 1;
  color: var(--color-on-surface, #111827);
  font-size: 13px;
  line-height: 1.5;
  overflow-wrap: anywhere;
}

:root.dark .entry-content {
  color: #f3f4f6;
}

.entry-meta {
  display: flex;
  justify-content: space-between;
  gap: 8px;
  overflow-wrap: anywhere;
}

.backup-list {
  display: flex;
  flex-direction: column;
  gap: 8px;
}

.backup-item {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
  padding: 10px 12px;
  border: 1px solid var(--color-border, rgba(128, 128, 128, 0.15));
  border-radius: 8px;
  background: var(--color-container, rgba(255, 255, 255, 0.5));
}

:root.dark .backup-item {
  border-color: rgba(255, 255, 255, 0.08);
  background: rgba(24, 24, 28, 0.5);
}

.backup-main {
  min-width: 0;
}

.backup-name {
  color: var(--color-on-surface, #111827);
  font-size: 13px;
  font-weight: 500;
  overflow-wrap: anywhere;
}

:root.dark .backup-name {
  color: #f3f4f6;
}

.empty-backups {
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  min-height: 140px;
  color: var(--color-on-surface-muted, #9ca3af);
}
</style>
