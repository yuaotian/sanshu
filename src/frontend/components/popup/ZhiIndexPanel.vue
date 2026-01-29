<script setup lang="ts">
/**
 * ZhiIndexPanel - zhi 弹窗索引状态折叠面板
 *
 * 功能：
 * 1. 收起状态：显示状态图标 + 同步状态 + 已索引文件数 + 最后同步时间
 * 2. 展开状态：
 *    - 嵌套项目展示：检测到 Git 子仓库时分组显示
 *    - 统计卡片 + 同步操作按钮
 * 3. 智能降级：根据 sou 启用状态和 ACE 配置状态显示不同引导
 */
import { invoke } from '@tauri-apps/api/core'
import { computed, h, onMounted, ref, watch } from 'vue'
import type { NestedProjectInfo, ProjectIndexStatus, ProjectWithNestedStatus } from '../../types/tauri'

// ==================== Props & Emits ====================

interface Props {
  // 项目根路径（为空时完全隐藏面板）
  projectRoot: string | undefined
  // sou 代码搜索工具是否启用
  souEnabled: boolean
  // ACE 配置是否完整（base_url 和 token 均已配置）
  acemcpConfigured: boolean
  // 当前项目索引状态
  projectStatus: ProjectIndexStatus | null
  // 是否正在索引中
  isIndexing?: boolean
  // 同步操作是否加载中
  resyncLoading?: boolean
}

interface Emits {
  // 打开 MCP 工具设置页
  'open-settings': []
  // 打开索引详情 Modal
  'open-detail': []
  // 触发同步操作（增量/全量）
  'resync': [type: 'incremental' | 'full']
}

const props = withDefaults(defineProps<Props>(), {
  isIndexing: false,
  resyncLoading: false,
})

const emit = defineEmits<Emits>()

// ==================== 响应式状态 ====================

// 面板是否展开
const isExpanded = ref(false)

// 同步下拉菜单是否显示
const showSyncMenu = ref(false)

// 嵌套项目状态
const nestedStatus = ref<ProjectWithNestedStatus | null>(null)
const loadingNested = ref(false)
// 记录嵌套项目加载错误（用于前端显性提示）
const nestedError = ref<string | null>(null)

// ==================== 计算属性 ====================

// 是否应该显示面板（需要有项目路径）
const shouldShow = computed(() => !!props.projectRoot)

// 面板显示模式：normal（正常）/ guide-sou（引导启用 sou）/ guide-ace（引导配置 ACE）
const displayMode = computed<'normal' | 'guide-sou' | 'guide-ace'>(() => {
  if (!props.souEnabled)
    return 'guide-sou'
  if (!props.acemcpConfigured)
    return 'guide-ace'
  return 'normal'
})

// 是否有嵌套项目
const hasNestedProjects = computed(() => {
  return (nestedStatus.value?.nested_projects?.length ?? 0) > 0
})

// 嵌套项目列表
const nestedProjects = computed(() => nestedStatus.value?.nested_projects ?? [])

// 状态图标类名
const statusIcon = computed(() => {
  const status = props.projectStatus?.status
  switch (status) {
    case 'idle':
      return 'i-carbon-circle-dash text-gray-400'
    case 'indexing':
      return 'i-carbon-in-progress text-emerald-400/80 animate-spin'
    case 'synced':
      return 'i-carbon-checkmark-filled text-emerald-400'
    case 'failed':
      return 'i-carbon-warning-filled text-rose-400'
    default:
      return 'i-carbon-help text-gray-400'
  }
})

// 状态文案
const statusText = computed(() => {
  const status = props.projectStatus?.status
  switch (status) {
    case 'idle':
      return '空闲'
    case 'indexing':
      return `索引中 ${props.projectStatus?.progress || 0}%`
    case 'synced':
      return '已同步'
    case 'failed':
      return '索引失败'
    default:
      return '未知'
  }
})

// 文件总数（包含嵌套项目时汇总）
const totalFiles = computed(() => {
  if (hasNestedProjects.value && nestedStatus.value) {
    // 汇总主项目和所有嵌套项目的文件数
    let total = nestedStatus.value.root_status.total_files
    for (const np of nestedStatus.value.nested_projects) {
      if (np.index_status)
        total += np.index_status.total_files
    }
    return total
  }
  return props.projectStatus?.total_files ?? 0
})

// 已索引文件数
const indexedFiles = computed(() => {
  if (hasNestedProjects.value && nestedStatus.value) {
    let indexed = nestedStatus.value.root_status.indexed_files
    for (const np of nestedStatus.value.nested_projects) {
      if (np.index_status)
        indexed += np.index_status.indexed_files
    }
    return indexed
  }
  return props.projectStatus?.indexed_files ?? 0
})

// 待处理文件数（包含嵌套项目时汇总）
const pendingFiles = computed(() => {
  if (hasNestedProjects.value && nestedStatus.value) {
    let pending = nestedStatus.value.root_status.pending_files
    for (const np of nestedStatus.value.nested_projects) {
      if (np.index_status)
        pending += np.index_status.pending_files
    }
    return pending
  }
  return props.projectStatus?.pending_files ?? 0
})

// 失败文件数（包含嵌套项目时汇总）
const failedFiles = computed(() => {
  if (hasNestedProjects.value && nestedStatus.value) {
    let failed = nestedStatus.value.root_status.failed_files
    for (const np of nestedStatus.value.nested_projects) {
      if (np.index_status)
        failed += np.index_status.failed_files
    }
    return failed
  }
  return props.projectStatus?.failed_files ?? 0
})

// 格式化最后同步时间
const lastSyncTime = computed(() => {
  const time = props.projectStatus?.last_success_time
  if (!time)
    return null

  try {
    const syncDate = new Date(time)
    const now = new Date()
    const diffMs = now.getTime() - syncDate.getTime()
    const diffMinutes = Math.floor(diffMs / 60000)

    if (diffMinutes < 1)
      return '刚刚'
    if (diffMinutes < 60)
      return `${diffMinutes}分钟前`
    const diffHours = Math.floor(diffMinutes / 60)
    if (diffHours < 24)
      return `${diffHours}小时前`
    const diffDays = Math.floor(diffHours / 24)
    return `${diffDays}天前`
  }
  catch {
    return time
  }
})

// 是否正在执行同步操作
const isSyncing = computed(() => props.resyncLoading || props.isIndexing)

// 项目根目录名称（仅显示最后一段）
const projectName = computed(() => {
  if (!props.projectRoot)
    return null
  // 兼容 Windows 和 Unix 路径分隔符，并去除末尾分隔符
  const normalized = props.projectRoot.replace(/\\/g, '/').replace(/\/+$/, '')
  if (!normalized)
    return null
  const segments = normalized.split('/')
  return segments[segments.length - 1] || null
})

// 最近增量索引的文件列表
const recentIndexedFiles = computed(() => {
  const files = props.projectStatus?.recent_indexed_files ?? []
  const normalized: string[] = []
  const seen = new Set<string>()
  for (const file of files) {
    // 去除 chunk 后缀，避免展示为 blob 片段
    const base = file.split('#chunk')[0] || file
    if (!seen.has(base)) {
      seen.add(base)
      normalized.push(base)
    }
  }
  return normalized
})

// 最近索引文件的显示文本
const recentFilesText = computed(() => {
  const files = recentIndexedFiles.value
  if (files.length === 0)
    return null

  const firstName = files[0].split('/').pop() || files[0]
  if (files.length === 1)
    return firstName
  return `${firstName} 等 ${files.length} 个`
})

// ==================== 事件处理 ====================

// 加载嵌套项目状态
async function fetchNestedStatus() {
  if (!props.projectRoot)
    return

  loadingNested.value = true
  nestedError.value = null
  try {
    const result = await invoke<ProjectWithNestedStatus>('get_acemcp_project_with_nested', {
      projectRootPath: props.projectRoot,
    })
    nestedStatus.value = result
  }
  catch (err) {
    console.error('获取嵌套项目状态失败:', err)
    nestedError.value = String(err)
  }
  finally {
    loadingNested.value = false
  }
}

// 切换面板展开状态
function toggleExpand() {
  isExpanded.value = !isExpanded.value
  // 展开时加载嵌套项目状态
  if (isExpanded.value && !nestedStatus.value)
    fetchNestedStatus()
}

// 处理同步操作
function handleResync(type: 'incremental' | 'full') {
  showSyncMenu.value = false
  emit('resync', type)
}

// 打开设置页面
function handleOpenSettings() {
  emit('open-settings')
}

// 打开索引详情 Modal
function handleOpenDetail() {
  emit('open-detail')
}

// 获取子项目状态图标
function getNestedStatusIcon(np: NestedProjectInfo): string {
  const status = np.index_status?.status
  switch (status) {
    case 'synced':
      return 'i-carbon-checkmark-filled text-emerald-400'
    case 'indexing':
      return 'i-carbon-in-progress text-emerald-400/80 animate-spin'
    case 'failed':
      return 'i-carbon-warning-filled text-rose-400'
    default:
      return 'i-carbon-circle-dash text-gray-400/60'
  }
}

// 获取子项目状态文字
function getNestedStatusText(np: NestedProjectInfo): string {
  const status = np.index_status
  if (!status)
    return '未索引'
  return `${status.indexed_files}/${status.total_files}`
}

// 监听项目路径变化，重新加载嵌套状态
watch(() => props.projectRoot, () => {
  nestedStatus.value = null
  nestedError.value = null
  if (isExpanded.value)
    fetchNestedStatus()
})

// 初始化
onMounted(() => {
  // 如果默认展开，加载嵌套状态
  if (isExpanded.value)
    fetchNestedStatus()
})
</script>

<template>
  <!-- 仅当有项目路径时显示面板 -->
  <div v-if="shouldShow" class="zhi-index-panel">
    <!-- ==================== 引导模式：sou 未启用 ==================== -->
    <div
      v-if="displayMode === 'guide-sou'"
      class="panel-guide"
    >
      <div class="guide-icon">
        <div class="i-carbon-search text-lg text-gray-400/80" />
      </div>
      <div class="guide-content">
        <span class="guide-text">启用代码搜索以使用智能索引</span>
        <n-button text type="primary" size="tiny" @click="handleOpenSettings">
          前往设置
          <template #icon>
            <div class="i-carbon-arrow-right" />
          </template>
        </n-button>
      </div>
    </div>

    <!-- ==================== 引导模式：ACE 未配置 ==================== -->
    <div
      v-else-if="displayMode === 'guide-ace'"
      class="panel-guide"
    >
      <div class="guide-icon guide-icon--warning">
        <div class="i-carbon-api text-lg text-amber-400/80" />
      </div>
      <div class="guide-content">
        <span class="guide-text">配置 API 密钥以启用代码索引</span>
        <n-button text type="primary" size="tiny" @click="handleOpenSettings">
          前往配置
          <template #icon>
            <div class="i-carbon-arrow-right" />
          </template>
        </n-button>
      </div>
    </div>

    <!-- ==================== 正常模式：索引状态面板 ==================== -->
    <div v-else class="panel-normal">
      <!-- 收起状态条 -->
      <div class="panel-header" @click="toggleExpand">
        <div class="header-left">
          <!-- 状态图标 -->
          <div :class="statusIcon" class="status-icon" />
          <!-- 状态文案 -->
          <span class="status-text">{{ statusText }}</span>
          <!-- 分隔符 -->
          <span class="status-divider">·</span>
          <!-- 文件数（显示嵌套项目数量提示） -->
          <span class="status-files">
            已索引 {{ indexedFiles }}/{{ totalFiles }} 个文件
            <span v-if="hasNestedProjects" class="nested-badge">
              含 {{ nestedProjects.length }} 个子项目
            </span>
          </span>
          <!-- 最后同步时间（如有） -->
          <template v-if="lastSyncTime">
            <span class="status-divider">·</span>
            <span class="status-time">上次同步 {{ lastSyncTime }}</span>
          </template>
        </div>
        <div class="header-right flex items-center gap-2">
          <!-- 最近索引文件信息（响应式隐藏） -->
          <n-tooltip v-if="recentFilesText" trigger="hover" :delay="300">
            <template #trigger>
              <div class="hidden md:flex items-center gap-1 text-white/50 text-[11px] max-w-[100px]">
                <div class="i-carbon-document shrink-0 text-white/40 text-xs" />
                <span class="truncate">{{ recentFilesText }}</span>
              </div>
            </template>
            <div class="flex flex-col gap-1 max-w-[280px]">
              <div v-for="(file, idx) in recentIndexedFiles.slice(0, 5)" :key="idx" class="text-xs truncate">
                {{ file }}
              </div>
              <div class="text-[10px] text-white/40 mt-1 pt-1 border-t border-white/10">
                {{ recentIndexedFiles.length > 5
                  ? `共 ${recentIndexedFiles.length} 个文件，仅显示最近 5 个`
                  : '最近增量索引的文件' }}
              </div>
            </div>
          </n-tooltip>
          <!-- 分隔符 -->
          <span v-if="recentFilesText && projectName" class="text-white/25 text-xs hidden md:inline">·</span>
          <!-- 项目根目录名称 -->
          <n-tooltip v-if="projectName" trigger="hover" :delay="300">
            <template #trigger>
              <div class="flex items-center gap-1.5 text-white/55 text-xs max-w-[120px]">
                <div class="i-carbon-folder shrink-0 text-white/40 text-sm" />
                <span class="truncate">{{ projectName }}</span>
              </div>
            </template>
            <span class="text-xs">{{ props.projectRoot }}</span>
          </n-tooltip>
          <!-- 分隔符 -->
          <span v-if="projectName" class="text-white/25 text-xs">·</span>
          <!-- 展开/收起图标 -->
          <div
            class="expand-icon"
            :class="isExpanded ? 'i-carbon-chevron-up' : 'i-carbon-chevron-down'"
          />
        </div>
      </div>

      <!-- 展开内容区域 -->
      <n-collapse-transition :show="isExpanded">
        <div class="panel-content">
          <!-- 嵌套项目区域（如果有） -->
          <div v-if="hasNestedProjects || nestedError" class="nested-projects-section">
            <div class="section-header">
              <div class="i-carbon-folder-parent section-icon" />
              <span class="section-title">Git 子项目</span>
            </div>
            <!-- 骨架屏 -->
            <div v-if="loadingNested" class="nested-skeleton">
              <div v-for="i in 3" :key="i" class="skeleton-item">
                <div class="skeleton-icon" />
                <div class="skeleton-text" />
              </div>
            </div>
            <!-- 错误提示 -->
            <div v-else-if="nestedError" class="nested-error">
              <div class="i-carbon-warning-alt" />
              <span>{{ nestedError }}</span>
            </div>
            <!-- 嵌套项目列表 -->
            <div v-else class="nested-list">
              <div
                v-for="np in nestedProjects"
                :key="np.absolute_path"
                class="nested-item"
              >
                <div class="nested-item__left">
                  <div class="i-carbon-folder-details nested-item__folder-icon" />
                  <span class="nested-item__name">{{ np.relative_path }}</span>
                </div>
                <div class="nested-item__right">
                  <span class="nested-item__stats">{{ getNestedStatusText(np) }}</span>
                  <div :class="getNestedStatusIcon(np)" class="nested-item__status-icon" />
                </div>
              </div>
            </div>
          </div>

          <!-- 统计卡片网格 -->
          <div class="stats-grid">
            <!-- 总文件数 -->
            <div class="stat-card">
              <div class="stat-value">{{ totalFiles }}</div>
              <div class="stat-label">总文件</div>
            </div>
            <!-- 已索引 -->
            <div class="stat-card stat-card--success">
              <div class="stat-value">{{ indexedFiles }}</div>
              <div class="stat-label">已索引</div>
            </div>
            <!-- 待处理 -->
            <div class="stat-card stat-card--info">
              <div class="stat-value">{{ pendingFiles }}</div>
              <div class="stat-label">待处理</div>
            </div>
            <!-- 失败 -->
            <div class="stat-card stat-card--error">
              <div class="stat-value">{{ failedFiles }}</div>
              <div class="stat-label">失败</div>
            </div>
          </div>

          <!-- 操作按钮区域 -->
          <div class="actions-row">
            <!-- 同步按钮（带下拉菜单） -->
            <n-dropdown
              :show="showSyncMenu"
              trigger="click"
              placement="bottom-start"
              :options="[
                { label: '增量同步', key: 'incremental', icon: () => h('div', { class: 'i-carbon-restart' }) },
                { label: '全量重建', key: 'full', icon: () => h('div', { class: 'i-carbon-renew' }) },
              ]"
              @select="handleResync"
              @clickoutside="showSyncMenu = false"
            >
              <n-button
                size="small"
                :loading="isSyncing"
                :disabled="isSyncing"
                @click="showSyncMenu = !showSyncMenu"
              >
                <template #icon>
                  <div class="i-carbon-sync" />
                </template>
                {{ isSyncing ? '同步中...' : '同步' }}
                <div class="i-carbon-chevron-down ml-1 text-xs" />
              </n-button>
            </n-dropdown>

            <!-- 查看详情按钮 -->
            <n-button text size="small" @click="handleOpenDetail">
              <template #icon>
                <div class="i-carbon-document-view" />
              </template>
              查看详情
            </n-button>
          </div>
        </div>
      </n-collapse-transition>
    </div>
  </div>
</template>

<style scoped>
/* ==================== 面板容器 ==================== */
.zhi-index-panel {
  margin: 8px;
  border-radius: 12px;
  overflow: hidden;
  background: linear-gradient(135deg, rgba(30, 30, 30, 0.7) 0%, rgba(25, 25, 25, 0.8) 100%);
  border: 1px solid rgba(255, 255, 255, 0.06);
  backdrop-filter: blur(12px);
  box-shadow: 0 2px 8px rgba(0, 0, 0, 0.15);
}

/* ==================== 引导模式样式 ==================== */
.panel-guide {
  display: flex;
  align-items: center;
  gap: 12px;
  padding: 12px 16px;
}

.guide-icon {
  display: flex;
  align-items: center;
  justify-content: center;
  width: 36px;
  height: 36px;
  border-radius: 10px;
  background: linear-gradient(135deg, rgba(255, 255, 255, 0.04) 0%, rgba(255, 255, 255, 0.02) 100%);
  border: 1px solid rgba(255, 255, 255, 0.06);
}

.guide-icon--warning {
  background: linear-gradient(135deg, rgba(251, 191, 36, 0.08) 0%, rgba(251, 191, 36, 0.04) 100%);
  border-color: rgba(251, 191, 36, 0.15);
}

.guide-content {
  flex: 1;
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 8px;
}

.guide-text {
  font-size: 12px;
  color: rgba(255, 255, 255, 0.65);
}

/* ==================== 正常模式 - 头部状态条 ==================== */
.panel-header {
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: 12px 16px;
  min-height: 44px;
  cursor: pointer;
  transition: background 0.2s ease;
}

.panel-header:hover {
  background: rgba(255, 255, 255, 0.02);
}

.header-left {
  display: flex;
  align-items: center;
  flex-wrap: wrap;
  gap: 6px;
  row-gap: 4px;
  font-size: 12px;
}

.status-icon {
  width: 14px;
  height: 14px;
  flex-shrink: 0;
}

.status-text {
  color: rgba(255, 255, 255, 0.9);
  font-weight: 500;
}

.status-divider {
  color: rgba(255, 255, 255, 0.25);
}

.status-files {
  color: rgba(255, 255, 255, 0.65);
}

.nested-badge {
  display: inline-flex;
  align-items: center;
  padding: 1px 6px;
  margin-left: 4px;
  border-radius: 4px;
  font-size: 10px;
  background: linear-gradient(135deg, rgba(52, 211, 153, 0.15) 0%, rgba(52, 211, 153, 0.08) 100%);
  color: rgba(52, 211, 153, 0.9);
  border: 1px solid rgba(52, 211, 153, 0.2);
}

.status-time {
  color: rgba(255, 255, 255, 0.45);
}

.header-right {
  display: flex;
  align-items: center;
}

.expand-icon {
  width: 14px;
  height: 14px;
  color: rgba(255, 255, 255, 0.4);
  transition: transform 0.2s ease;
}

/* ==================== 正常模式 - 展开内容 ==================== */
.panel-content {
  padding: 0 16px 16px;
  border-top: 1px solid rgba(255, 255, 255, 0.04);
}

/* ==================== 嵌套项目区域 ==================== */
.nested-projects-section {
  margin-top: 12px;
  padding: 10px 12px;
  border-radius: 10px;
  background: linear-gradient(135deg, rgba(52, 211, 153, 0.04) 0%, rgba(52, 211, 153, 0.02) 100%);
  border: 1px solid rgba(52, 211, 153, 0.1);
}

.section-header {
  display: flex;
  align-items: center;
  gap: 6px;
  margin-bottom: 10px;
}

.section-icon {
  width: 14px;
  height: 14px;
  color: rgba(52, 211, 153, 0.7);
}

.section-title {
  font-size: 11px;
  font-weight: 500;
  color: rgba(52, 211, 153, 0.9);
  text-transform: uppercase;
  letter-spacing: 0.5px;
}

/* 骨架屏 */
.nested-skeleton {
  display: flex;
  flex-direction: column;
  gap: 8px;
}

/* 嵌套项目错误提示 */
.nested-error {
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 8px 10px;
  border-radius: 8px;
  background: rgba(248, 113, 113, 0.08);
  color: rgba(248, 113, 113, 0.9);
  font-size: 12px;
}

.skeleton-item {
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 6px 0;
}

.skeleton-icon {
  width: 14px;
  height: 14px;
  border-radius: 4px;
  background: rgba(255, 255, 255, 0.08);
  animation: skeleton-pulse 1.5s ease-in-out infinite;
}

.skeleton-text {
  height: 12px;
  width: 80px;
  border-radius: 4px;
  background: rgba(255, 255, 255, 0.08);
  animation: skeleton-pulse 1.5s ease-in-out infinite;
}

@keyframes skeleton-pulse {
  0%, 100% { opacity: 1; }
  50% { opacity: 0.4; }
}

/* 嵌套项目列表 */
.nested-list {
  display: flex;
  flex-direction: column;
  gap: 4px;
}

.nested-item {
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: 6px 8px;
  border-radius: 6px;
  background: rgba(255, 255, 255, 0.02);
  transition: background 0.2s ease;
}

.nested-item:hover {
  background: rgba(255, 255, 255, 0.04);
}

.nested-item__left {
  display: flex;
  align-items: center;
  gap: 8px;
}

.nested-item__folder-icon {
  width: 14px;
  height: 14px;
  color: rgba(52, 211, 153, 0.6);
}

.nested-item__name {
  font-size: 12px;
  font-weight: 500;
  color: rgba(255, 255, 255, 0.85);
}

.nested-item__right {
  display: flex;
  align-items: center;
  gap: 6px;
}

.nested-item__stats {
  font-size: 11px;
  color: rgba(255, 255, 255, 0.5);
  font-family: ui-monospace, monospace;
}

.nested-item__status-icon {
  width: 12px;
  height: 12px;
}

/* ==================== 统计卡片网格 ==================== */
.stats-grid {
  display: grid;
  grid-template-columns: repeat(4, minmax(0, 1fr));
  gap: 8px;
  margin-top: 12px;
}

.stat-card {
  display: flex;
  flex-direction: column;
  align-items: center;
  padding: 10px 8px;
  border-radius: 8px;
  background: linear-gradient(135deg, rgba(255, 255, 255, 0.04) 0%, rgba(255, 255, 255, 0.02) 100%);
  border: 1px solid rgba(255, 255, 255, 0.05);
  transition: all 0.2s ease;
}

.stat-card:hover {
  background: linear-gradient(135deg, rgba(255, 255, 255, 0.06) 0%, rgba(255, 255, 255, 0.03) 100%);
  border-color: rgba(255, 255, 255, 0.08);
}

.stat-card--success .stat-value {
  color: #86efac;
}

.stat-card--info .stat-value {
  color: #93c5fd;
}

.stat-card--error .stat-value {
  color: #fca5a5;
}

.stat-value {
  font-size: 16px;
  font-weight: 600;
  color: rgba(255, 255, 255, 0.9);
  line-height: 1.2;
}

.stat-label {
  font-size: 10px;
  color: rgba(255, 255, 255, 0.45);
  margin-top: 2px;
}

/* ==================== 操作按钮区域 ==================== */
.actions-row {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
  margin-top: 12px;
  padding-top: 12px;
  border-top: 1px solid rgba(255, 255, 255, 0.04);
}
</style>
