<script setup lang="ts">
/**
 * ZhiIndexPanel - zhi 弹窗索引状态折叠面板
 * 
 * 功能：
 * 1. 收起状态：显示状态图标 + 同步状态 + Blob 数 + 最后同步时间
 * 2. 展开状态：4格统计卡片 + 同步操作按钮
 * 3. 智能降级：根据 sou 启用状态和 ACE 配置状态显示不同引导
 */
import type { ProjectIndexStatus } from '../../types/tauri'
import { computed, h, ref } from 'vue'

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

// ==================== 计算属性 ====================

// 是否应该显示面板（需要有项目路径）
const shouldShow = computed(() => !!props.projectRoot)

// 面板显示模式：normal（正常）/ guide-sou（引导启用 sou）/ guide-ace（引导配置 ACE）
const displayMode = computed<'normal' | 'guide-sou' | 'guide-ace'>(() => {
  if (!props.souEnabled) return 'guide-sou'
  if (!props.acemcpConfigured) return 'guide-ace'
  return 'normal'
})

// 状态图标类名
const statusIcon = computed(() => {
  const status = props.projectStatus?.status
  switch (status) {
    case 'idle':
      return 'i-carbon-circle-dash text-gray-400'
    case 'indexing':
      return 'i-carbon-in-progress text-blue-400 animate-spin'
    case 'synced':
      return 'i-carbon-checkmark-filled text-green-400'
    case 'failed':
      return 'i-carbon-warning-filled text-red-400'
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

// 文件总数
const totalFiles = computed(() => props.projectStatus?.total_files ?? 0)

// 已索引文件数
const indexedFiles = computed(() => props.projectStatus?.indexed_files ?? 0)

// 待处理文件数
const pendingFiles = computed(() => props.projectStatus?.pending_files ?? 0)

// 失败文件数
const failedFiles = computed(() => props.projectStatus?.failed_files ?? 0)

// 格式化最后同步时间
const lastSyncTime = computed(() => {
  const time = props.projectStatus?.last_success_time
  if (!time) return null

  // 尝试计算相对时间
  try {
    const syncDate = new Date(time)
    const now = new Date()
    const diffMs = now.getTime() - syncDate.getTime()
    const diffMinutes = Math.floor(diffMs / 60000)

    if (diffMinutes < 1) return '刚刚'
    if (diffMinutes < 60) return `${diffMinutes}分钟前`
    const diffHours = Math.floor(diffMinutes / 60)
    if (diffHours < 24) return `${diffHours}小时前`
    const diffDays = Math.floor(diffHours / 24)
    return `${diffDays}天前`
  }
  catch {
    return time
  }
})

// 是否正在执行同步操作
const isSyncing = computed(() => props.resyncLoading || props.isIndexing)

// ==================== 事件处理 ====================

// 切换面板展开状态
function toggleExpand() {
  isExpanded.value = !isExpanded.value
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
        <div class="i-carbon-search text-lg text-gray-400" />
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
      <div class="guide-icon">
        <div class="i-carbon-api text-lg text-amber-400" />
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
          <!-- 文件数 -->
          <span class="status-files">已索引 {{ indexedFiles }}/{{ totalFiles }} 个文件</span>
          <!-- 最后同步时间（如有） -->
          <template v-if="lastSyncTime">
            <span class="status-divider">·</span>
            <span class="status-time">上次同步 {{ lastSyncTime }}</span>
          </template>
        </div>
        <div class="header-right">
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
            <!-- 待处理（仅在有值时显示） -->
            <div class="stat-card stat-card--info">
              <div class="stat-value">{{ pendingFiles }}</div>
              <div class="stat-label">待处理</div>
            </div>
            <!-- 失败（仅在有值时显示） -->
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
                { label: '增量同步（推荐）', key: 'incremental', icon: () => h('div', { class: 'i-carbon-restart' }) },
                { label: '全量重建（清空索引）', key: 'full', icon: () => h('div', { class: 'i-carbon-renew' }) },
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
  border-radius: 10px;
  overflow: hidden;
  background: rgba(30, 30, 30, 0.6);
  border: 1px solid rgba(255, 255, 255, 0.08);
  backdrop-filter: blur(8px);
}

/* ==================== 引导模式样式 ==================== */
.panel-guide {
  display: flex;
  align-items: center;
  gap: 10px;
  padding: 10px 14px;
}

.guide-icon {
  display: flex;
  align-items: center;
  justify-content: center;
  width: 32px;
  height: 32px;
  border-radius: 8px;
  background: rgba(255, 255, 255, 0.05);
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
  color: rgba(255, 255, 255, 0.7);
}

/* ==================== 正常模式 - 头部状态条 ==================== */
.panel-normal {
  /* 容器样式由父级处理 */
}

.panel-header {
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: 10px 14px;
  cursor: pointer;
  transition: background 0.2s;
}

.panel-header:hover {
  background: rgba(255, 255, 255, 0.03);
}

.header-left {
  display: flex;
  align-items: center;
  gap: 6px;
  font-size: 12px;
}

.status-icon {
  width: 14px;
  height: 14px;
}

.status-text {
  color: rgba(255, 255, 255, 0.9);
  font-weight: 500;
}

.status-divider {
  color: rgba(255, 255, 255, 0.3);
}

.status-files {
  color: rgba(255, 255, 255, 0.7);
}

.status-time {
  color: rgba(255, 255, 255, 0.5);
}

.header-right {
  display: flex;
  align-items: center;
}

.expand-icon {
  width: 14px;
  height: 14px;
  color: rgba(255, 255, 255, 0.5);
  transition: transform 0.2s;
}

/* ==================== 正常模式 - 展开内容 ==================== */
.panel-content {
  padding: 0 14px 14px;
  border-top: 1px solid rgba(255, 255, 255, 0.06);
}

/* 统计卡片网格 */
.stats-grid {
  display: grid;
  grid-template-columns: repeat(auto-fit, minmax(60px, 1fr));
  gap: 8px;
  margin-top: 12px;
}

.stat-card {
  display: flex;
  flex-direction: column;
  align-items: center;
  padding: 10px 8px;
  border-radius: 8px;
  background: rgba(255, 255, 255, 0.04);
  border: 1px solid rgba(255, 255, 255, 0.06);
}

.stat-card--success .stat-value {
  color: #4ade80;
}

.stat-card--info .stat-value {
  color: #60a5fa;
}

.stat-card--error .stat-value {
  color: #f87171;
}

.stat-value {
  font-size: 16px;
  font-weight: 600;
  color: rgba(255, 255, 255, 0.9);
  line-height: 1.2;
}

.stat-label {
  font-size: 10px;
  color: rgba(255, 255, 255, 0.5);
  margin-top: 2px;
}

/* 操作按钮区域 */
.actions-row {
  display: flex;
  align-items: center;
  justify-content: space-between;
  margin-top: 12px;
  padding-top: 10px;
  border-top: 1px solid rgba(255, 255, 255, 0.06);
}
</style>
