<script setup lang="ts">
import type { ProjectIndexStatus } from '../../types/tauri'
import { computed } from 'vue'

interface Props {
  project: ProjectIndexStatus
  isWatching: boolean
  watchInherited?: boolean
  // 目录是否存在
  directoryExists?: boolean
}

interface Emits {
  (e: 'view-tree'): void
  (e: 'reindex'): void
  (e: 'toggle-watching'): void
  // 复制路径事件，向父组件传递规范化后的路径
  (e: 'copy-path', path: string): void
  // 删除项目索引记录
  (e: 'delete'): void
}

const props = withDefaults(defineProps<Props>(), {
  directoryExists: true,
  watchInherited: false,
})
const emit = defineEmits<Emits>()

const isStale = computed(() => !!props.project.is_stale && props.project.status !== 'indexing')
const staleNotice = computed(() => {
  if (!isStale.value)
    return ''
  return props.project.stale_reason || '检测到 ACE 配置已变更，等待重新索引'
})
const workspaceResolutionNotice = computed(() => props.project.workspace_resolution_error || '')
const deleteActionLabel = computed(() => props.project.is_workspace ? '移除工作区监听与聚合视图' : '删除索引记录')

// 状态配置映射
const statusConfig = computed(() => {
  if (isStale.value) {
    return {
      text: '待重建',
      type: 'warning' as const,
      icon: 'i-carbon-warning-alt',
      glowColor: 'rgba(245, 158, 11, 0.35)',
      borderColor: 'border-amber-500/40',
    }
  }

  const configs = {
    idle: {
      text: '未索引',
      type: 'default' as const,
      icon: 'i-carbon-circle-dash',
      glowColor: 'rgba(156, 163, 175, 0.3)',
      borderColor: 'border-gray-400/30',
    },
    indexing: {
      text: '索引中',
      type: 'info' as const,
      icon: 'i-carbon-in-progress animate-spin',
      glowColor: 'rgba(59, 130, 246, 0.4)',
      borderColor: 'border-blue-500/40',
    },
    paused: {
      text: '等待恢复',
      type: 'warning' as const,
      icon: 'i-carbon-pause-outline',
      glowColor: 'rgba(245, 158, 11, 0.3)',
      borderColor: 'border-amber-500/40',
    },
    synced: {
      text: '已完成',
      type: 'success' as const,
      icon: 'i-carbon-checkmark-filled',
      glowColor: 'rgba(34, 197, 94, 0.3)',
      borderColor: 'border-green-500/30',
    },
    failed: {
      text: '失败',
      type: 'error' as const,
      icon: 'i-carbon-warning-filled',
      glowColor: 'rgba(239, 68, 68, 0.4)',
      borderColor: 'border-red-500/40',
    },
  }
  return configs[props.project.status] || configs.idle
})

// 规范化展示路径（去掉 Windows 扩展前缀并统一斜杠）
const displayPath = computed(() => {
  // 原始路径可能包含 Windows 扩展前缀，如 \\?\E:\\ 或 //?/E:/
  let p = props.project.project_root || ''
  if (p.startsWith('\\\\?\\'))
    p = p.slice(4)
  else if (p.startsWith('//?/'))
    p = p.slice(4)

  // 统一使用正斜杠，便于展示
  return p.replace(/\\/g, '/')
})

// 提取项目名称（路径最后一段）
const projectName = computed(() => {
  const parts = displayPath.value.split('/')
  return parts[parts.length - 1] || displayPath.value
})

// 最近成功同步的文件用于排查监听是否真正上传了变更。
const recentIndexedFiles = computed(() => props.project.recent_indexed_files?.slice(0, 5) || [])

// 格式化相对时间
function formatRelativeTime(timeStr: string | null): string {
  if (!timeStr)
    return '从未'
  try {
    const date = new Date(timeStr)
    const now = new Date()
    const diffMs = now.getTime() - date.getTime()
    const diffSec = Math.floor(diffMs / 1000)
    const diffMin = Math.floor(diffSec / 60)
    const diffHour = Math.floor(diffMin / 60)
    const diffDay = Math.floor(diffHour / 24)

    if (diffSec < 60)
      return '刚刚'
    if (diffMin < 60)
      return `${diffMin} 分钟前`
    if (diffHour < 24)
      return `${diffHour} 小时前`
    if (diffDay < 30)
      return `${diffDay} 天前`
    return date.toLocaleDateString('zh-CN')
  }
  catch {
    return '未知'
  }
}

// 格式化绝对时间
function formatAbsoluteTime(timeStr: string | null): string {
  if (!timeStr)
    return '从未索引'
  try {
    return new Date(timeStr).toLocaleString('zh-CN', {
      year: 'numeric',
      month: '2-digit',
      day: '2-digit',
      hour: '2-digit',
      minute: '2-digit',
      second: '2-digit',
    })
  }
  catch {
    return '时间格式错误'
  }
}
</script>

<template>
  <div
    class="project-card group"
    :class="[statusConfig.borderColor]"
    :style="{ '--glow-color': statusConfig.glowColor }"
  >
    <!-- 科技感扫描线动画（仅索引中时显示） -->
    <div v-if="project.status === 'indexing'" class="scan-line" />

    <!-- 顶部装饰线 -->
    <div class="card-top-border" />

    <div class="card-content">
      <!-- 头部：项目名称和状态 -->
      <div class="card-header">
        <div class="project-info">
          <!-- 项目名称 -->
          <div class="project-name">
            <div :class="project.is_workspace ? 'i-carbon-folder-details' : 'i-carbon-folder'" class="text-primary-500 flex-shrink-0" />
            <span class="name-text">{{ projectName }}</span>
            <n-tag v-if="project.is_workspace" size="tiny" :bordered="false" type="info">
              工作区 · {{ project.workspace_project_count || 0 }} 项目
            </n-tag>
            <!-- 目录不存在警告 -->
            <n-tooltip v-if="!props.directoryExists" trigger="hover">
              <template #trigger>
                <div class="i-carbon-warning-filled text-red-500 flex-shrink-0 ml-1" />
              </template>
              目录不存在，建议删除此记录
            </n-tooltip>
          </div>
          <!-- 项目路径 -->
          <n-tooltip trigger="hover">
            <template #trigger>
              <div
                class="project-path"
                @click="emit('copy-path', displayPath)"
              >
                {{ displayPath }}
              </div>
            </template>
            点击复制路径
          </n-tooltip>
        </div>

        <!-- 状态徽章 -->
        <n-tag
          :type="statusConfig.type"
          :bordered="false"
          size="small"
          class="status-badge"
        >
          <template #icon>
            <div class="text-xs" :class="[statusConfig.icon]" />
          </template>
          {{ statusConfig.text }}
        </n-tag>
      </div>

      <div v-if="staleNotice" class="stale-section">
        <div class="i-carbon-warning-alt stale-section__icon" />
        <span>{{ staleNotice }}</span>
      </div>

      <div v-if="workspaceResolutionNotice" class="resolution-error-section">
        <div class="i-carbon-warning-filled stale-section__icon" />
        <span>工作区解析失败：{{ workspaceResolutionNotice }}</span>
      </div>

      <div
        v-if="project.is_workspace && ((project.workspace_indexing_project_count || 0) > 0 || (project.workspace_paused_project_count || 0) > 0 || (project.workspace_failed_project_count || 0) > 0)"
        class="workspace-health"
      >
        <n-tag v-if="(project.workspace_indexing_project_count || 0) > 0" size="small" :bordered="false" type="info">
          {{ project.workspace_indexing_project_count }} 个索引中
        </n-tag>
        <n-tag v-if="(project.workspace_paused_project_count || 0) > 0" size="small" :bordered="false" type="warning">
          {{ project.workspace_paused_project_count }} 个等待恢复
        </n-tag>
        <n-tag v-if="(project.workspace_failed_project_count || 0) > 0" size="small" :bordered="false" type="error">
          {{ project.workspace_failed_project_count }} 个失败
        </n-tag>
      </div>

      <div v-if="recentIndexedFiles.length > 0" class="recent-files-section">
        <div class="recent-files-header">
          <div class="i-carbon-upload text-green-500" />
          <span>最近成功同步</span>
        </div>
        <div class="recent-files-list">
          <span v-for="file in recentIndexedFiles" :key="file" class="recent-file">
            {{ file }}
          </span>
        </div>
      </div>

      <!-- 进度条（仅索引中时显示） -->
      <div v-if="project.status === 'indexing' || project.status === 'paused'" class="progress-section">
        <n-progress
          type="line"
          :percentage="project.progress"
          :show-indicator="true"
          :height="6"
          :border-radius="3"
          :processing="project.status === 'indexing'"
          :status="project.status === 'paused' ? 'warning' : 'info'"
          class="cyber-progress"
        />
      </div>

      <!-- 文件统计 -->
      <div class="stats-section">
        <n-tooltip trigger="hover">
          <template #trigger>
            <div class="stat-item">
              <div class="i-carbon-document" />
              <span class="stat-label">总计</span>
              <span class="stat-value">{{ project.total_files }}</span>
            </div>
          </template>
          项目中的总文件数
        </n-tooltip>

        <n-tooltip trigger="hover">
          <template #trigger>
            <div class="stat-item text-green-500">
              <div class="i-carbon-checkmark-filled" />
              <span class="stat-label">已索引</span>
              <span class="stat-value">{{ project.indexed_files }}</span>
            </div>
          </template>
          已成功索引的文件数
        </n-tooltip>

        <n-tooltip v-if="project.pending_files > 0" trigger="hover">
          <template #trigger>
            <div class="stat-item text-blue-500">
              <div class="i-carbon-time" />
              <span class="stat-label">待处理</span>
              <span class="stat-value">{{ project.pending_files }}</span>
            </div>
          </template>
          等待索引的文件数
        </n-tooltip>

        <n-tooltip v-if="project.failed_files > 0" trigger="hover">
          <template #trigger>
            <div class="stat-item text-red-500">
              <div class="i-carbon-warning-filled" />
              <span class="stat-label">失败</span>
              <span class="stat-value">{{ project.failed_files }}</span>
            </div>
          </template>
          索引失败的文件数
        </n-tooltip>
      </div>

      <!-- 最后索引时间 -->
      <div class="time-section">
        <n-tooltip trigger="hover">
          <template #trigger>
            <div class="time-info">
              <div class="i-carbon-time" />
              <span>{{ formatRelativeTime(project.last_success_time) }}</span>
            </div>
          </template>
          {{ formatAbsoluteTime(project.last_success_time) }}
        </n-tooltip>
      </div>

      <!-- 操作按钮 -->
      <div class="actions-section">
        <!-- 监听开关 -->
        <n-tooltip trigger="hover">
          <template #trigger>
            <div class="watch-toggle">
              <n-switch
                :value="isWatching"
                size="small"
                :disabled="watchInherited"
                @update:value="emit('toggle-watching')"
              >
                <template #checked>
                  <div class="i-carbon-view text-[10px]" />
                </template>
                <template #unchecked>
                  <div class="i-carbon-view-off text-[10px]" />
                </template>
              </n-switch>
              <span class="watch-label">{{ watchInherited ? '工作区监听' : '监听' }}</span>
            </div>
          </template>
          {{ watchInherited ? '由父工作区统一监听并路由到当前子项目' : isWatching ? '停止 MCP 持久监听' : '开启 MCP 持久监听' }}
        </n-tooltip>

        <div class="flex-1" />

        <!-- 重新索引 -->
        <n-button
          size="tiny"
          secondary
          type="primary"
          :disabled="project.status === 'indexing'"
          @click="emit('reindex')"
        >
          <template #icon>
            <div class="i-carbon-renew text-xs" />
          </template>
          索引
        </n-button>

        <!-- 查看结构树 -->
        <n-button
          size="tiny"
          secondary
          type="info"
          @click="emit('view-tree')"
        >
          <template #icon>
            <div class="i-carbon-tree-view text-xs" />
          </template>
          结构
        </n-button>

        <!-- 删除按钮 -->
        <n-tooltip trigger="hover">
          <template #trigger>
            <n-button
              size="tiny"
              quaternary
              type="error"
              @click="emit('delete')"
            >
              <template #icon>
                <div class="i-carbon-trash-can text-xs" />
              </template>
            </n-button>
          </template>
          {{ deleteActionLabel }}
        </n-tooltip>
      </div>
    </div>
  </div>
</template>

<style scoped>
/* 项目卡片基础样式 */
.project-card {
  position: relative;
  border-radius: 12px;
  overflow: hidden;
  transition: all 0.3s cubic-bezier(0.4, 0, 0.2, 1);
  border: 1px solid;
  background: var(--color-container, rgba(255, 255, 255, 0.8));
  backdrop-filter: blur(8px);
}

/* 深色模式背景 */
:root.dark .project-card {
  background: rgba(24, 24, 28, 0.9);
}

/* 悬停效果 - 霓虹光晕 */
.project-card:hover {
  transform: translateY(-2px);
  box-shadow:
    0 8px 25px -5px var(--glow-color, rgba(0, 0, 0, 0.1)),
    0 0 20px -5px var(--glow-color, rgba(0, 0, 0, 0.1));
}

/* 卡片内容区域 */
.card-content {
  padding: 16px;
  display: flex;
  flex-direction: column;
  gap: 12px;
}

/* 头部区域 */
.card-header {
  display: flex;
  align-items: flex-start;
  justify-content: space-between;
  gap: 12px;
}

/* 项目信息 */
.project-info {
  flex: 1;
  min-width: 0;
}

/* 项目名称 */
.project-name {
  display: flex;
  align-items: center;
  gap: 8px;
  margin-bottom: 4px;
}

.name-text {
  font-weight: 500;
  font-size: 14px;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

/* 项目路径 */
.project-path {
  font-size: 11px;
  font-family: ui-monospace, monospace;
  opacity: 0.5;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  cursor: pointer;
  transition: opacity 0.2s ease;
}

.project-path:hover {
  opacity: 0.8;
}

/* 状态徽章 */
.status-badge {
  flex-shrink: 0;
}

/* 进度条区域 */
.progress-section {
  position: relative;
}

/* 统计信息区域 */
.stats-section {
  display: flex;
  flex-wrap: wrap;
  gap: 8px;
  font-size: 12px;
}

/* 时间信息区域 */
.time-section {
  font-size: 11px;
}

.time-info {
  display: flex;
  align-items: center;
  gap: 4px;
  opacity: 0.5;
}

.stale-section {
  display: flex;
  align-items: flex-start;
  gap: 6px;
  padding: 10px 12px;
  border-radius: 10px;
  font-size: 11px;
  line-height: 1.5;
  color: #b45309;
  background: rgba(245, 158, 11, 0.12);
  border: 1px solid rgba(245, 158, 11, 0.24);
}

.stale-section__icon {
  flex-shrink: 0;
  margin-top: 1px;
}

.resolution-error-section {
  display: flex;
  align-items: flex-start;
  gap: 6px;
  padding: 10px 12px;
  border: 1px solid rgba(239, 68, 68, 0.24);
  border-radius: 8px;
  color: #b91c1c;
  background: rgba(239, 68, 68, 0.1);
  font-size: 11px;
  line-height: 1.5;
}

:root.dark .resolution-error-section {
  border-color: rgba(248, 113, 113, 0.3);
  color: #fca5a5;
  background: rgba(239, 68, 68, 0.16);
}

.workspace-health {
  display: flex;
  flex-wrap: wrap;
  gap: 6px;
}

:root.dark .stale-section {
  color: #fcd34d;
  background: rgba(245, 158, 11, 0.18);
  border-color: rgba(245, 158, 11, 0.3);
}

.recent-files-section {
  padding: 10px 12px;
  border-radius: 8px;
  background: rgba(34, 197, 94, 0.08);
  border: 1px solid rgba(34, 197, 94, 0.18);
}

.recent-files-header {
  display: flex;
  align-items: center;
  gap: 6px;
  margin-bottom: 8px;
  font-size: 11px;
  font-weight: 500;
  color: #15803d;
}

.recent-files-list {
  display: flex;
  flex-direction: column;
  gap: 4px;
}

.recent-file {
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  font-family: ui-monospace, monospace;
  font-size: 10px;
  line-height: 1.4;
  color: rgba(22, 101, 52, 0.88);
}

:root.dark .recent-files-section {
  background: rgba(34, 197, 94, 0.12);
  border-color: rgba(34, 197, 94, 0.24);
}

:root.dark .recent-files-header {
  color: #86efac;
}

:root.dark .recent-file {
  color: rgba(187, 247, 208, 0.88);
}

/* 操作按钮区域 */
.actions-section {
  display: flex;
  align-items: center;
  gap: 8px;
  padding-top: 12px;
  border-top: 1px solid rgba(128, 128, 128, 0.2);
}

/* 监听开关 */
.watch-toggle {
  display: flex;
  align-items: center;
  gap: 6px;
}

.watch-label {
  font-size: 10px;
  opacity: 0.6;
}

/* 顶部装饰线 - 渐变霓虹效果 */
.card-top-border {
  position: absolute;
  top: 0;
  left: 0;
  right: 0;
  height: 2px;
  background: linear-gradient(
    90deg,
    transparent,
    var(--glow-color, rgba(59, 130, 246, 0.5)),
    transparent
  );
  opacity: 0;
  transition: opacity 0.3s ease;
}

.project-card:hover .card-top-border {
  opacity: 1;
}

/* 扫描线动画 */
.scan-line {
  position: absolute;
  top: 0;
  left: 0;
  right: 0;
  height: 100%;
  background: linear-gradient(
    180deg,
    transparent 0%,
    rgba(59, 130, 246, 0.1) 50%,
    transparent 100%
  );
  animation: scan 2s linear infinite;
  pointer-events: none;
  z-index: 1;
}

@keyframes scan {
  0% {
    transform: translateY(-100%);
  }
  100% {
    transform: translateY(100%);
  }
}

/* 统计项样式 */
.stat-item {
  display: inline-flex;
  align-items: center;
  gap: 4px;
  padding: 3px 8px;
  border-radius: 4px;
  background: rgba(128, 128, 128, 0.08);
  transition: all 0.2s ease;
}

.stat-item:hover {
  background: rgba(128, 128, 128, 0.15);
}

/* 统计标签 - 确保可见 */
.stat-label {
  font-size: 10px;
  opacity: 0.7;
}

/* 统计数值 - 加粗显示 */
.stat-value {
  font-weight: 600;
  font-variant-numeric: tabular-nums;
}

/* 科技感进度条 */
.cyber-progress :deep(.n-progress-graph-line-fill) {
  background: linear-gradient(90deg, #3b82f6, #8b5cf6, #3b82f6);
  background-size: 200% 100%;
  animation: gradient-flow 2s linear infinite;
}

@keyframes gradient-flow {
  0% {
    background-position: 0% 50%;
  }
  100% {
    background-position: 200% 50%;
  }
}
</style>
