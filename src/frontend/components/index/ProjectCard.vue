<script setup lang="ts">
import type { ProjectIndexStatus } from '../../types/tauri'
import { computed } from 'vue'

interface Props {
  project: ProjectIndexStatus
  isWatching: boolean
}

interface Emits {
	(e: 'view-tree'): void
	(e: 'reindex'): void
	(e: 'toggle-watching'): void
	// 复制路径事件，向父组件传递规范化后的路径
	(e: 'copy-path', path: string): void
}

const props = defineProps<Props>()
const emit = defineEmits<Emits>()

// 状态配置映射
const statusConfig = computed(() => {
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

// 格式化相对时间
function formatRelativeTime(timeStr: string | null): string {
  if (!timeStr) return '从未'
  try {
    const date = new Date(timeStr)
    const now = new Date()
    const diffMs = now.getTime() - date.getTime()
    const diffSec = Math.floor(diffMs / 1000)
    const diffMin = Math.floor(diffSec / 60)
    const diffHour = Math.floor(diffMin / 60)
    const diffDay = Math.floor(diffHour / 24)

    if (diffSec < 60) return '刚刚'
    if (diffMin < 60) return `${diffMin} 分钟前`
    if (diffHour < 24) return `${diffHour} 小时前`
    if (diffDay < 30) return `${diffDay} 天前`
    return date.toLocaleDateString('zh-CN')
  } catch {
    return '未知'
  }
}

// 格式化绝对时间
function formatAbsoluteTime(timeStr: string | null): string {
  if (!timeStr) return '从未索引'
  try {
    return new Date(timeStr).toLocaleString('zh-CN', {
      year: 'numeric',
      month: '2-digit',
      day: '2-digit',
      hour: '2-digit',
      minute: '2-digit',
      second: '2-digit',
    })
  } catch {
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
            <div class="i-carbon-folder text-primary-500 flex-shrink-0" />
            <span class="name-text">{{ projectName }}</span>
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
            <div :class="[statusConfig.icon, 'text-xs']" />
          </template>
          {{ statusConfig.text }}
        </n-tag>
      </div>

      <!-- 进度条（仅索引中时显示） -->
      <div v-if="project.status === 'indexing'" class="progress-section">
        <n-progress
          type="line"
          :percentage="project.progress"
          :show-indicator="true"
          :height="6"
          :border-radius="3"
          processing
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
                @update:value="emit('toggle-watching')"
              >
                <template #checked>
                  <div class="i-carbon-view text-[10px]" />
                </template>
                <template #unchecked>
                  <div class="i-carbon-view-off text-[10px]" />
                </template>
              </n-switch>
              <span class="watch-label">监听</span>
            </div>
          </template>
          {{ isWatching ? '停止实时监听' : '开启实时监听' }}
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

