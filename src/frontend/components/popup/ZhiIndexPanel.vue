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
  'open-settings': [toolId?: string]
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
// 命令不可用时不再重复请求，避免反复报错
const nestedCommandUnavailable = ref(false)
const nestedCommandUnavailableMessage = '当前后端版本不支持“Git 子项目”功能，请升级客户端。'

// ==================== 计算属性 ====================

// 引导模式不依赖项目路径，normal 模式才需要
const shouldShow = computed(() => {
  if (displayMode.value !== 'normal')
    return true
  return !!props.projectRoot
})

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

const isAuthFailure = computed(() => {
  if (props.projectStatus?.status !== 'failed')
    return false

  const lastError = props.projectStatus?.last_error || ''
  const lower = lastError.toLowerCase()
  return lower.includes('401') || lower.includes('认证失败') || lower.includes('invalid token')
})

const authFailureMessage = computed(() => {
  return props.projectStatus?.last_error || 'ACE API 认证失败 (401)：Token 已失效或被封禁，请在设置中更新 Token'
})

const authFailureHint = '更新完成后，可重新同步或等待自动索引恢复'

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
      return isAuthFailure.value
        ? 'i-carbon-warning-alt-filled text-rose-400'
        : 'i-carbon-warning-filled text-rose-400'
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
      return isAuthFailure.value ? 'ACE Token 已失效' : '索引失败'
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

  // 命令不可用时直接提示，避免反复请求
  if (nestedCommandUnavailable.value) {
    nestedError.value = nestedCommandUnavailableMessage
    return
  }

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
    const errorText = String(err)
    const lowerText = errorText.toLowerCase()
    if (lowerText.includes('get_acemcp_project_with_nested') && lowerText.includes('not found')) {
      nestedCommandUnavailable.value = true
      nestedError.value = nestedCommandUnavailableMessage
      return
    }
    nestedError.value = errorText
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
function handleOpenSettings(toolId?: string) {
  emit('open-settings', toolId)
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
  <div v-if="shouldShow" class="mx-2 mt-2">
    <n-alert
      v-if="displayMode === 'guide-sou'"
      type="default"
      :bordered="false"
      class="text-xs"
      :show-icon="false"
    >
      <div class="flex items-center justify-between gap-2">
        <div class="flex items-center gap-2">
          <div class="i-carbon-search w-3.5 h-3.5 text-on-surface-muted shrink-0" />
          <span>启用代码搜索以使用智能索引</span>
        </div>
        <n-button text type="primary" size="tiny" @click="handleOpenSettings('sou')">
          前往设置
          <template #icon>
            <div class="i-carbon-arrow-right" />
          </template>
        </n-button>
      </div>
    </n-alert>

    <n-alert
      v-else-if="displayMode === 'guide-ace'"
      type="warning"
      :bordered="false"
      class="text-xs"
      :show-icon="false"
    >
      <div class="flex items-center justify-between gap-2">
        <div class="flex items-center gap-2">
          <div class="i-carbon-api w-3.5 h-3.5 text-warning shrink-0" />
          <span>配置 API 密钥以启用代码索引</span>
        </div>
        <n-button text type="primary" size="tiny" @click="handleOpenSettings('sou')">
          前往配置
          <template #icon>
            <div class="i-carbon-arrow-right" />
          </template>
        </n-button>
      </div>
    </n-alert>

    <n-card v-else size="small" embedded>
      <div class="flex items-center justify-between cursor-pointer" @click="toggleExpand">
        <div class="flex items-center flex-wrap gap-1.5 text-xs">
          <div :class="statusIcon" class="w-3.5 h-3.5 shrink-0" />
          <n-text class="text-xs font-medium">{{ statusText }}</n-text>
          <n-text depth="3" class="text-xs">·</n-text>
          <n-text depth="3" class="text-xs">
            已索引 {{ indexedFiles }}/{{ totalFiles }} 个文件
            <n-tag v-if="hasNestedProjects" size="tiny" :bordered="false" type="success" class="ml-1">
              含 {{ nestedProjects.length }} 个子项目
            </n-tag>
          </n-text>
          <template v-if="lastSyncTime">
            <n-text depth="3" class="text-xs">·</n-text>
            <n-text depth="3" class="text-[11px]">上次同步 {{ lastSyncTime }}</n-text>
          </template>
        </div>
        <div class="flex items-center gap-2">
          <n-tooltip v-if="recentFilesText" trigger="hover" :delay="300">
            <template #trigger>
              <div class="hidden md:flex items-center gap-1 text-[11px] max-w-[100px]">
                <div class="i-carbon-document shrink-0 text-xs" />
                <n-text depth="3" class="truncate text-[11px]">{{ recentFilesText }}</n-text>
              </div>
            </template>
            <div class="flex flex-col gap-1 max-w-[280px]">
              <div v-for="(file, idx) in recentIndexedFiles.slice(0, 5)" :key="idx" class="text-xs truncate">
                {{ file }}
              </div>
              <n-text depth="3" class="text-[10px] mt-1 pt-1 border-t border-border block">
                {{ recentIndexedFiles.length > 5
                  ? `共 ${recentIndexedFiles.length} 个文件，仅显示最近 5 个`
                  : '最近增量索引的文件' }}
              </n-text>
            </div>
          </n-tooltip>
          <n-text v-if="recentFilesText && projectName" depth="3" class="text-xs hidden md:inline">·</n-text>
          <n-tooltip v-if="projectName" trigger="hover" :delay="300">
            <template #trigger>
              <div class="flex items-center gap-1.5 text-xs max-w-[120px]">
                <div class="i-carbon-folder shrink-0 text-sm" />
                <n-text depth="3" class="truncate text-xs">{{ projectName }}</n-text>
              </div>
            </template>
            <span class="text-xs">{{ props.projectRoot }}</span>
          </n-tooltip>
          <n-text v-if="projectName" depth="3" class="text-xs">·</n-text>
          <div
            class="w-3.5 h-3.5 transition-transform duration-200"
            :class="isExpanded ? 'i-carbon-chevron-up' : 'i-carbon-chevron-down'"
          />
        </div>
      </div>

      <n-collapse-transition :show="isExpanded">
        <div class="pt-3 mt-3 border-t border-border">
          <n-alert v-if="isAuthFailure" type="error" :bordered="false" class="mb-3">
            <template #header>
              ACE Token 已失效，请更新配置
            </template>
            <div class="text-xs">{{ authFailureMessage }}</div>
            <div v-if="authFailureHint" class="text-xs mt-1 opacity-80">{{ authFailureHint }}</div>
            <template #action>
              <n-button
                size="tiny"
                type="error"
                secondary
                @click.stop="handleOpenSettings('sou')"
              >
                <template #icon>
                  <div class="i-carbon-settings-adjust" />
                </template>
                前往设置更新 Token
              </n-button>
            </template>
          </n-alert>

          <n-card v-if="hasNestedProjects || nestedError" size="small" class="mb-3" embedded>
            <template #header>
              <div class="flex items-center gap-1.5 text-xs">
                <div class="i-carbon-folder-parent text-success w-3.5 h-3.5" />
                <span class="text-[11px] font-medium text-success uppercase tracking-wider">Git 子项目</span>
              </div>
            </template>
            <div v-if="loadingNested">
              <n-skeleton v-for="i in 3" :key="i" text class="mb-2" />
            </div>
            <n-alert v-else-if="nestedError" type="error" :bordered="false">
              {{ nestedError }}
            </n-alert>
            <div v-else class="flex flex-col gap-1">
              <div
                v-for="np in nestedProjects"
                :key="np.absolute_path"
                class="flex items-center justify-between py-1.5 px-2 rounded hover:bg-container-secondary transition-colors"
              >
                <div class="flex items-center gap-2">
                  <div class="i-carbon-folder-details w-3.5 h-3.5 text-success" />
                  <n-text class="text-xs font-medium">{{ np.relative_path }}</n-text>
                </div>
                <div class="flex items-center gap-1.5">
                  <n-text depth="3" class="text-[11px] font-mono">{{ getNestedStatusText(np) }}</n-text>
                  <div :class="getNestedStatusIcon(np)" class="w-3 h-3" />
                </div>
              </div>
            </div>
          </n-card>

          <n-grid :cols="4" :x-gap="8" class="mb-3">
            <n-grid-item>
              <n-card size="small" embedded class="text-center">
                <div class="text-base font-semibold">{{ totalFiles }}</div>
                <n-text depth="3" class="text-[10px]">总文件</n-text>
              </n-card>
            </n-grid-item>
            <n-grid-item>
              <n-card size="small" embedded class="text-center">
                <div class="text-base font-semibold text-success">{{ indexedFiles }}</div>
                <n-text depth="3" class="text-[10px]">已索引</n-text>
              </n-card>
            </n-grid-item>
            <n-grid-item>
              <n-card size="small" embedded class="text-center">
                <div class="text-base font-semibold text-info">{{ pendingFiles }}</div>
                <n-text depth="3" class="text-[10px]">待处理</n-text>
              </n-card>
            </n-grid-item>
            <n-grid-item>
              <n-card size="small" embedded class="text-center">
                <div class="text-base font-semibold text-error">{{ failedFiles }}</div>
                <n-text depth="3" class="text-[10px]">失败</n-text>
              </n-card>
            </n-grid-item>
          </n-grid>

          <div class="flex items-center justify-between pt-3 border-t border-border">
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
            <n-button text size="small" @click="handleOpenDetail">
              <template #icon>
                <div class="i-carbon-document-view" />
              </template>
              查看详情
            </n-button>
          </div>
        </div>
      </n-collapse-transition>
    </n-card>
  </div>
</template>

