<script setup lang="ts">
/**
 * 记忆管理配置组件
 * 包含：配置设置、记忆列表、相似度预览
 */
import { invoke } from '@tauri-apps/api/core';
import { useMessage } from 'naive-ui';
import { computed, onMounted, ref, watch } from 'vue';
import ConfigSection from '../common/ConfigSection.vue';

// Props
const props = defineProps<{
  active: boolean
  projectRootPath?: string | null
}>()

const message = useMessage()

// ============ 类型定义 ============
interface MemoryEntry {
  id: string
  content: string
  category: string
  created_at: string
}

interface MemoryConfig {
  similarity_threshold: number
  dedup_on_startup: boolean
  enable_dedup: boolean
}

interface MemoryStats {
  total: number
  rules: number
  preferences: number
  patterns: number
  contexts: number
}

interface DedupResult {
  original_count: number
  removed_count: number
  remaining_count: number
  removed_ids: string[]
}

interface SimilarityPreview {
  is_duplicate: boolean
  similarity: number
  matched_id: string | null
  matched_content: string | null
  threshold: number
}

// ============ 状态 ============
const currentTab = ref('config')
const loading = ref(false)
const projectPath = computed(() => props.projectRootPath || '')

// 配置状态
const config = ref<MemoryConfig>({
  similarity_threshold: 0.70,
  dedup_on_startup: true,
  enable_dedup: true,
})
const configLoading = ref(false)
const configSaving = ref(false)

// 记忆列表状态
const memories = ref<MemoryEntry[]>([])
const stats = ref<MemoryStats>({ total: 0, rules: 0, preferences: 0, patterns: 0, contexts: 0 })
const listLoading = ref(false)
const expandedCategories = ref<string[]>(['规范', '偏好', '模式', '背景'])

// 去重状态
const dedupLoading = ref(false)
const lastDedupResult = ref<DedupResult | null>(null)

// 相似度预览状态
const previewContent = ref('')
const previewLoading = ref(false)
const previewResult = ref<SimilarityPreview | null>(null)

// 删除确认状态
const deleteConfirmId = ref<string | null>(null)
const deleteLoading = ref(false)

// ============ 计算属性 ============
const groupedMemories = computed(() => {
  const groups: Record<string, MemoryEntry[]> = {
    '规范': [],
    '偏好': [],
    '模式': [],
    '背景': [],
  }
  for (const m of memories.value) {
    if (groups[m.category]) {
      groups[m.category].push(m)
    }
  }
  return groups
})

const thresholdPercent = computed({
  get: () => Math.round(config.value.similarity_threshold * 100),
  set: (val: number) => {
    config.value.similarity_threshold = val / 100
  },
})

// ============ 加载函数 ============
async function loadConfig() {
  if (!projectPath.value) return
  configLoading.value = true
  try {
    const res = await invoke<MemoryConfig>('get_memory_config', { projectPath: projectPath.value })
    config.value = res
  }
  catch (err) {
    message.error(`加载配置失败: ${err}`)
  }
  finally {
    configLoading.value = false
  }
}

async function loadMemories() {
  if (!projectPath.value) return
  listLoading.value = true
  try {
    const [memoryList, memoryStats] = await Promise.all([
      invoke<MemoryEntry[]>('get_memory_list', { projectPath: projectPath.value }),
      invoke<MemoryStats>('get_memory_stats', { projectPath: projectPath.value }),
    ])
    memories.value = memoryList
    stats.value = memoryStats
  }
  catch (err) {
    message.error(`加载记忆列表失败: ${err}`)
  }
  finally {
    listLoading.value = false
  }
}

// ============ 操作函数 ============
async function saveConfig() {
  if (!projectPath.value) return
  configSaving.value = true
  try {
    await invoke('save_memory_config', {
      projectPath: projectPath.value,
      config: config.value,
    })
    message.success('配置已保存')
  }
  catch (err) {
    message.error(`保存配置失败: ${err}`)
  }
  finally {
    configSaving.value = false
  }
}

async function executeDeduplicate() {
  if (!projectPath.value) return
  dedupLoading.value = true
  try {
    const result = await invoke<DedupResult>('deduplicate_memories', { projectPath: projectPath.value })
    lastDedupResult.value = result
    if (result.removed_count > 0) {
      message.success(`去重完成：移除 ${result.removed_count} 条重复记忆`)
      await loadMemories()
    }
    else {
      message.info('未发现重复记忆')
    }
  }
  catch (err) {
    message.error(`去重失败: ${err}`)
  }
  finally {
    dedupLoading.value = false
  }
}

async function previewSimilarity() {
  if (!projectPath.value || !previewContent.value.trim()) {
    message.warning('请输入要检测的内容')
    return
  }
  previewLoading.value = true
  try {
    const result = await invoke<SimilarityPreview>('preview_similarity', {
      projectPath: projectPath.value,
      content: previewContent.value,
    })
    previewResult.value = result
  }
  catch (err) {
    message.error(`预览失败: ${err}`)
  }
  finally {
    previewLoading.value = false
  }
}

async function deleteMemory(id: string) {
  if (!projectPath.value) return
  deleteLoading.value = true
  try {
    await invoke('delete_memory', { projectPath: projectPath.value, memoryId: id })
    message.success('记忆已删除')
    deleteConfirmId.value = null
    await loadMemories()
  }
  catch (err) {
    message.error(`删除失败: ${err}`)
  }
  finally {
    deleteLoading.value = false
  }
}

function formatDate(isoString: string): string {
  try {
    return new Date(isoString).toLocaleString('zh-CN')
  }
  catch {
    return isoString
  }
}

function getCategoryIcon(category: string): string {
  const icons: Record<string, string> = {
    '规范': 'i-carbon-rule',
    '偏好': 'i-carbon-user-favorite',
    '模式': 'i-carbon-flow',
    '背景': 'i-carbon-document',
  }
  return icons[category] || 'i-carbon-document'
}

function getCategoryColor(category: string): string {
  const colors: Record<string, string> = {
    '规范': 'text-info',
    '偏好': 'text-primary',
    '模式': 'text-success',
    '背景': 'text-warning',
  }
  return colors[category] || 'text-on-surface-muted'
}

// ============ 生命周期 ============
watch(() => props.active, async (active) => {
  if (active && projectPath.value) {
    loading.value = true
    await Promise.all([loadConfig(), loadMemories()])
    loading.value = false
  }
})

onMounted(async () => {
  if (props.active && projectPath.value) {
    loading.value = true
    await Promise.all([loadConfig(), loadMemories()])
    loading.value = false
  }
})
</script>

<template>
  <div>
    <!-- 无项目路径提示 -->
    <div
      v-if="!projectPath"
      class="flex flex-col items-center justify-center min-h-[200px] text-on-surface-muted"
    >
      <div class="i-carbon-folder-off text-5xl mb-3 opacity-20" />
      <div class="text-sm opacity-60">
        请先在 MCP 工具中指定项目路径
      </div>
    </div>

    <template v-else>
      <n-tabs v-model:value="currentTab" type="line" animated>
        <!-- 配置 Tab -->
        <n-tab-pane name="config" tab="配置">
          <n-scrollbar class="max-h-[500px]">
            <n-space vertical size="medium" class="py-4 px-1">
              <!-- 去重设置 -->
              <ConfigSection title="去重设置" description="配置相似度检测阈值和自动去重行为">
                <n-space vertical size="medium">
                  <!-- 相似度阈值滑块 -->
                  <div>
                    <div class="text-xs text-on-surface-secondary mb-1">
                      相似度阈值
                    </div>
                    <div class="w-full">
                      <div class="flex items-center gap-4">
                        <n-slider
                          v-model:value="thresholdPercent"
                          :min="50"
                          :max="95"
                          :step="5"
                          :marks="{ 50: '50%', 70: '70%', 95: '95%' }"
                          class="flex-1"
                        />
                        <n-tag type="info" size="small" :bordered="false">
                          {{ thresholdPercent }}%
                        </n-tag>
                      </div>
                      <div class="text-xs text-on-surface-muted mt-1">
                        超过此相似度的内容将被视为重复。建议值：70%
                      </div>
                    </div>
                  </div>

                  <!-- 开关选项 -->
                  <n-space vertical size="medium">
                    <n-card size="small">
                      <div class="flex items-center justify-between gap-4">
                        <div class="flex-1 min-w-0">
                          <div class="text-sm font-medium text-on-surface">
                            启动时自动去重
                          </div>
                          <div class="text-xs text-on-surface-secondary mt-0.5">
                            每次加载记忆时自动检测并移除重复内容
                          </div>
                        </div>
                        <n-switch v-model:value="config.dedup_on_startup" size="small" />
                      </div>
                    </n-card>
                    <n-card size="small">
                      <div class="flex items-center justify-between gap-4">
                        <div class="flex-1 min-w-0">
                          <div class="text-sm font-medium text-on-surface">
                            启用去重检测
                          </div>
                          <div class="text-xs text-on-surface-secondary mt-0.5">
                            添加新记忆时检测是否与现有内容重复
                          </div>
                        </div>
                        <n-switch v-model:value="config.enable_dedup" size="small" />
                      </div>
                    </n-card>
                  </n-space>
                </n-space>
              </ConfigSection>

              <!-- 快捷操作 -->
              <ConfigSection title="快捷操作" :no-card="true">
                <n-space>
                  <n-button type="primary" size="small" :loading="configSaving" @click="saveConfig">
                    <template #icon>
                      <div class="i-carbon-save" />
                    </template>
                    保存配置
                  </n-button>
                  <n-button secondary size="small" :loading="dedupLoading" @click="executeDeduplicate">
                    <template #icon>
                      <div class="i-carbon-clean" />
                    </template>
                    立即整理
                  </n-button>
                </n-space>
              </ConfigSection>

              <!-- 统计信息 -->
              <ConfigSection title="统计信息" :no-card="true">
                <div class="grid grid-cols-5 gap-3 w-full">
                  <n-card size="small" class="text-center">
                    <div class="text-2xl font-semibold text-on-surface">
                      {{ stats.total }}
                    </div>
                    <div class="text-xs text-on-surface-secondary mt-1">
                      总计
                    </div>
                  </n-card>
                  <n-card size="small" class="text-center">
                    <div class="text-2xl font-semibold text-info">
                      {{ stats.rules }}
                    </div>
                    <div class="text-xs text-on-surface-secondary mt-1">
                      规范
                    </div>
                  </n-card>
                  <n-card size="small" class="text-center">
                    <div class="text-2xl font-semibold text-primary">
                      {{ stats.preferences }}
                    </div>
                    <div class="text-xs text-on-surface-secondary mt-1">
                      偏好
                    </div>
                  </n-card>
                  <n-card size="small" class="text-center">
                    <div class="text-2xl font-semibold text-success">
                      {{ stats.patterns }}
                    </div>
                    <div class="text-xs text-on-surface-secondary mt-1">
                      模式
                    </div>
                  </n-card>
                  <n-card size="small" class="text-center">
                    <div class="text-2xl font-semibold text-warning">
                      {{ stats.contexts }}
                    </div>
                    <div class="text-xs text-on-surface-secondary mt-1">
                      背景
                    </div>
                  </n-card>
                </div>
              </ConfigSection>

              <!-- 去重结果 -->
              <n-collapse-transition :show="lastDedupResult !== null">
                <ConfigSection v-if="lastDedupResult" title="上次整理结果" :no-card="true">
                  <n-alert type="success" :bordered="false">
                    <template #icon>
                      <div class="i-carbon-checkmark-outline" />
                    </template>
                    移除 <strong>{{ lastDedupResult.removed_count }}</strong> 条重复记忆，
                    保留 <strong>{{ lastDedupResult.remaining_count }}</strong> 条
                  </n-alert>
                </ConfigSection>
              </n-collapse-transition>
            </n-space>
          </n-scrollbar>
        </n-tab-pane>

        <!-- 记忆列表 Tab -->
        <n-tab-pane name="list" tab="记忆列表">
          <n-scrollbar class="max-h-[500px]">
            <n-space vertical size="medium" class="py-4 px-1">
              <!-- 加载骨架屏 -->
              <div v-if="listLoading" class="flex flex-col gap-4">
                <n-skeleton v-for="i in 4" :key="i" text :repeat="2" />
              </div>

              <!-- 空状态 -->
              <div
                v-else-if="memories.length === 0"
                class="flex flex-col items-center justify-center min-h-[200px] text-on-surface-muted"
              >
                <div class="i-carbon-document text-4xl mb-2 opacity-20" />
                <div class="text-sm opacity-60">暂无记忆条目</div>
              </div>

              <!-- 分组列表 -->
              <n-collapse v-else v-model:expanded-names="expandedCategories" arrow-placement="left">
                <n-collapse-item
                  v-for="(items, category) in groupedMemories"
                  :key="category"
                  :name="category"
                  :disabled="items.length === 0"
                >
                  <template #header>
                    <div class="flex items-center gap-2 font-medium">
                      <div :class="[getCategoryIcon(category), getCategoryColor(category)]" />
                      <span>{{ category }}</span>
                      <n-tag size="small" :bordered="false">{{ items.length }}</n-tag>
                    </div>
                  </template>

                  <div class="flex flex-col gap-2">
                    <n-card v-for="item in items" :key="item.id" size="small">
                      <div class="text-sm leading-normal text-on-surface break-words">
                        {{ item.content }}
                      </div>
                      <div class="flex items-center justify-between mt-2 pt-2 border-t border-border">
                        <span class="text-xs text-on-surface-muted">{{ formatDate(item.created_at) }}</span>
                        <n-popconfirm
                          :show="deleteConfirmId === item.id"
                          @positive-click="deleteMemory(item.id)"
                          @negative-click="deleteConfirmId = null"
                        >
                          <template #trigger>
                            <n-button
                              text
                              type="error"
                              size="tiny"
                              :loading="deleteLoading && deleteConfirmId === item.id"
                              @click="deleteConfirmId = item.id"
                            >
                              <template #icon>
                                <div class="i-carbon-trash-can" />
                              </template>
                            </n-button>
                          </template>
                          确定要删除这条记忆吗？
                        </n-popconfirm>
                      </div>
                    </n-card>
                  </div>
                </n-collapse-item>
              </n-collapse>

              <!-- 刷新按钮 -->
              <div class="flex justify-center pt-2">
                <n-button text type="primary" size="small" :loading="listLoading" @click="loadMemories">
                  <template #icon>
                    <div class="i-carbon-renew" />
                  </template>
                  刷新列表
                </n-button>
              </div>
            </n-space>
          </n-scrollbar>
        </n-tab-pane>

        <!-- 相似度预览 Tab -->
        <n-tab-pane name="preview" tab="相似度预览">
          <n-scrollbar class="max-h-[500px]">
            <n-space vertical size="medium" class="py-4 px-1">
              <ConfigSection title="输入检测" description="输入内容检测与现有记忆的相似度">
                <n-space vertical size="medium">
                  <n-input
                    v-model:value="previewContent"
                    type="textarea"
                    size="small"
                    :rows="3"
                    placeholder="输入要检测的内容..."
                  />
                  <n-button
                    type="primary"
                    size="small"
                    :loading="previewLoading"
                    :disabled="!previewContent.trim()"
                    @click="previewSimilarity"
                  >
                    <template #icon>
                      <div class="i-carbon-search" />
                    </template>
                    检测相似度
                  </n-button>
                </n-space>
              </ConfigSection>

              <!-- 检测结果 -->
              <n-collapse-transition :show="previewResult !== null">
                <ConfigSection v-if="previewResult" title="检测结果" :no-card="true">
                  <n-card size="small">
                    <!-- 相似度指示器 -->
                    <div class="relative h-6 rounded-xl bg-container-secondary overflow-hidden">
                      <div
                        class="similarity-bar"
                        :style="{ width: `${previewResult.similarity * 100}%` }"
                        :class="{
                          'bg-error': previewResult.is_duplicate,
                          'bg-success': !previewResult.is_duplicate,
                        }"
                      />
                      <div class="similarity-text">
                        相似度: {{ (previewResult.similarity * 100).toFixed(1) }}%
                        <span class="opacity-80">
                          (阈值: {{ (previewResult.threshold * 100).toFixed(0) }}%)
                        </span>
                      </div>
                    </div>

                    <!-- 结果状态 -->
                    <n-alert
                      :type="previewResult.is_duplicate ? 'warning' : 'success'"
                      :bordered="false"
                      class="mt-4"
                    >
                      <template #icon>
                        <div :class="previewResult.is_duplicate ? 'i-carbon-warning' : 'i-carbon-checkmark'" />
                      </template>
                      {{ previewResult.is_duplicate ? '检测到相似内容，添加时将被拒绝' : '未检测到相似内容，可以添加' }}
                    </n-alert>

                    <!-- 匹配的内容 -->
                    <div
                      v-if="previewResult.matched_content"
                      class="mt-4 p-3 rounded-lg bg-container-secondary"
                    >
                      <div class="text-xs text-on-surface-secondary mb-1">
                        最相似的记忆:
                      </div>
                      <div class="text-sm text-on-surface">
                        {{ previewResult.matched_content }}
                      </div>
                    </div>
                  </n-card>
                </ConfigSection>
              </n-collapse-transition>
            </n-space>
          </n-scrollbar>
        </n-tab-pane>
      </n-tabs>
    </template>
  </div>
</template>

<style scoped>
.similarity-bar {
  position: absolute;
  top: 0;
  left: 0;
  height: 100%;
  transition: width 0.3s ease;
}

.similarity-text {
  position: absolute;
  top: 50%;
  left: 50%;
  transform: translate(-50%, -50%);
  font-size: 12px;
  font-weight: 500;
  color: white;
  text-shadow: 0 1px 2px rgba(0, 0, 0, 0.3);
}
</style>
