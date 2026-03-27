<script setup lang="ts">
import { computed } from 'vue'
import type { ProjectIndexStatus } from '../../types/tauri'

interface Props {
  project: ProjectIndexStatus
  isWatching: boolean
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
})
const emit = defineEmits<Emits>()

const isStale = computed(() => !!props.project.is_stale && props.project.status !== 'indexing')
const staleNotice = computed(() => {
  if (!isStale.value)
    return ''
  return props.project.stale_reason || '检测到 ACE 配置已变更，等待重新索引'
})

// 状态配置映射
const statusConfig = computed(() => {
  if (isStale.value) {
    return {
      text: '待重建',
      type: 'warning' as const,
      icon: 'i-carbon-warning-alt',
      glowColor: '',
      borderColor: '',
    }
  }

  const configs = {
    idle: {
      text: '未索引',
      type: 'default' as const,
      icon: 'i-carbon-circle-dash',
      glowColor: '',
      borderColor: '',
    },
    indexing: {
      text: '索引中',
      type: 'info' as const,
      icon: 'i-carbon-in-progress animate-spin',
      glowColor: '',
      borderColor: '',
    },
    synced: {
      text: '已完成',
      type: 'success' as const,
      icon: 'i-carbon-checkmark-filled',
      glowColor: '',
      borderColor: '',
    },
    failed: {
      text: '失败',
      type: 'error' as const,
      icon: 'i-carbon-warning-filled',
      glowColor: '',
      borderColor: '',
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
  <n-card size="small" hoverable>
    <n-space vertical :size="8">
      <div class="flex items-start justify-between gap-3">
        <div class="flex-1 min-w-0">
          <div class="flex items-center gap-2 mb-1">
            <div class="i-carbon-folder text-primary shrink-0" />
            <n-text class="text-sm font-medium truncate">{{ projectName }}</n-text>
            <n-tooltip v-if="!props.directoryExists" trigger="hover">
              <template #trigger>
                <div class="i-carbon-warning-filled text-error shrink-0" />
              </template>
              目录不存在，建议删除此记录
            </n-tooltip>
          </div>
          <n-tooltip trigger="hover">
            <template #trigger>
              <n-text
                depth="3"
                class="text-[11px] font-mono truncate block cursor-pointer hover:opacity-80 transition-opacity"
                @click="emit('copy-path', displayPath)"
              >
                {{ displayPath }}
              </n-text>
            </template>
            点击复制路径
          </n-tooltip>
        </div>
        <n-tag :type="statusConfig.type" :bordered="false" size="small" class="shrink-0">
          <template #icon>
            <div class="text-xs" :class="[statusConfig.icon]" />
          </template>
          {{ statusConfig.text }}
        </n-tag>
      </div>

      <n-alert v-if="staleNotice" type="warning" :bordered="false" class="text-xs">
        {{ staleNotice }}
      </n-alert>

      <n-progress
        v-if="project.status === 'indexing'"
        type="line"
        :percentage="project.progress"
        :show-indicator="true"
        :height="6"
        :border-radius="3"
        processing
      />

      <n-space :size="6" class="text-xs">
        <n-tooltip trigger="hover">
          <template #trigger>
            <n-tag size="tiny" :bordered="false">
              <template #icon>
                <div class="i-carbon-document text-[10px]" />
              </template>
              总计 {{ project.total_files }}
            </n-tag>
          </template>
          项目中的总文件数
        </n-tooltip>
        <n-tooltip trigger="hover">
          <template #trigger>
            <n-tag size="tiny" type="success" :bordered="false">
              <template #icon>
                <div class="i-carbon-checkmark-filled text-[10px]" />
              </template>
              已索引 {{ project.indexed_files }}
            </n-tag>
          </template>
          已成功索引的文件数
        </n-tooltip>
        <n-tooltip v-if="project.pending_files > 0" trigger="hover">
          <template #trigger>
            <n-tag size="tiny" type="info" :bordered="false">
              <template #icon>
                <div class="i-carbon-time text-[10px]" />
              </template>
              待处理 {{ project.pending_files }}
            </n-tag>
          </template>
          等待索引的文件数
        </n-tooltip>
        <n-tooltip v-if="project.failed_files > 0" trigger="hover">
          <template #trigger>
            <n-tag size="tiny" type="error" :bordered="false">
              <template #icon>
                <div class="i-carbon-warning-filled text-[10px]" />
              </template>
              失败 {{ project.failed_files }}
            </n-tag>
          </template>
          索引失败的文件数
        </n-tooltip>
      </n-space>

      <n-tooltip trigger="hover">
        <template #trigger>
          <div class="flex items-center gap-1 text-[11px]">
            <div class="i-carbon-time" />
            <n-text depth="3">{{ formatRelativeTime(project.last_success_time) }}</n-text>
          </div>
        </template>
        {{ formatAbsoluteTime(project.last_success_time) }}
      </n-tooltip>

      <div class="flex items-center gap-2 pt-2 border-t border-border">
        <n-tooltip trigger="hover">
          <template #trigger>
            <div class="flex items-center gap-1.5">
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
              <n-text depth="3" class="text-[10px]">监听</n-text>
            </div>
          </template>
          {{ isWatching ? '停止实时监听' : '开启实时监听' }}
        </n-tooltip>
        <div class="flex-1" />
        <n-button size="tiny" secondary type="primary" :disabled="project.status === 'indexing'" @click="emit('reindex')">
          <template #icon>
            <div class="i-carbon-renew text-xs" />
          </template>
          索引
        </n-button>
        <n-button size="tiny" secondary type="info" @click="emit('view-tree')">
          <template #icon>
            <div class="i-carbon-tree-view text-xs" />
          </template>
          结构
        </n-button>
        <n-tooltip trigger="hover">
          <template #trigger>
            <n-button size="tiny" quaternary type="error" @click="emit('delete')">
              <template #icon>
                <div class="i-carbon-trash-can text-xs" />
              </template>
            </n-button>
          </template>
          删除索引记录
        </n-tooltip>
      </div>
    </n-space>
  </n-card>
</template>
