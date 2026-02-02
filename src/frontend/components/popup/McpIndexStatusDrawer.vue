<script setup lang="ts">
import { invoke } from '@tauri-apps/api/core'
import type { TreeOption } from 'naive-ui'
import { useMessage } from 'naive-ui'
import { computed, h, ref, watch } from 'vue'
import type { FileIndexStatusType, NestedProjectInfo, ProjectFilesStatus, ProjectIndexStatus, ProjectWithNestedStatus } from '../../types/tauri'

interface Props {
  show: boolean
  projectRoot: string
  statusSummary: string
  statusIcon: string
  projectStatus: ProjectIndexStatus | null
  isIndexing?: boolean
  resyncLoading?: boolean
}

interface Emits {
  'update:show': [value: boolean]
  'resync': []
}

const props = withDefaults(defineProps<Props>(), {
  isIndexing: false,
  resyncLoading: false,
})

const emit = defineEmits<Emits>()

// 弹窗显示状态，使用 v-model:show 双向绑定父组件
const modalVisible = computed({
  get: () => props.show,
  set: (val: boolean) => emit('update:show', val),
})

// 规范化展示路径（去掉 Windows 扩展前缀并统一斜杠）
const displayPath = computed(() => {
  let p = props.projectRoot || ''
  // 处理 Windows 扩展路径前缀 \\?\ 或 //?/
  if (p.startsWith('\\\\?\\'))
    p = p.slice(4)
  else if (p.startsWith('//?/'))
    p = p.slice(4)
  // 统一使用正斜杠
  return p.replace(/\\/g, '/')
})

// 提取项目名称
const projectName = computed(() => {
  const parts = displayPath.value.split('/')
  return parts[parts.length - 1] || displayPath.value
})

// 文件索引状态数据
const filesStatus = ref<ProjectFilesStatus | null>(null)
const loadingFiles = ref(false)
const filesError = ref<string | null>(null)
// 是否仅显示未完全同步的文件（过滤开关）
const showOnlyPending = ref(false)

// 嵌套项目状态
const nestedStatus = ref<ProjectWithNestedStatus | null>(null)
const loadingNested = ref(false)
// 记录嵌套项目加载错误（用于前端显性提示）
const nestedError = ref<string | null>(null)
// 命令不可用时不再重复请求，避免反复报错
const nestedCommandUnavailable = ref(false)
const nestedCommandUnavailableMessage = '当前后端版本不支持“Git 子项目”功能，请升级客户端。'

const message = useMessage()

// Tree 节点类型
type NodeStatus = 'indexed' | 'pending'

// 扩展的树节点接口，包含图标渲染所需的额外信息
interface IndexTreeNode {
  key: string
  label: string
  children?: IndexTreeNode[]
  // 仅文件节点使用的状态
  status?: NodeStatus
  // 是否为目录节点
  isDirectory?: boolean
  // 文件扩展名（用于图标映射）
  fileExtension?: string
  // 原始文件名（不含状态后缀）
  fileName?: string
}

// ==================== 文件图标映射系统 ====================

// 文件图标配置接口
interface FileIconConfig {
  icon: string
  color: string
}

// 文件扩展名到图标的映射表
const FILE_ICON_MAP: Record<string, FileIconConfig> = {
  // Rust
  rs: { icon: 'i-carbon-code', color: '#dea584' },
  // Vue
  vue: { icon: 'i-carbon-application', color: '#42b883' },
  // TypeScript
  ts: { icon: 'i-carbon-code', color: '#3178c6' },
  tsx: { icon: 'i-carbon-code', color: '#3178c6' },
  // JavaScript
  js: { icon: 'i-carbon-code', color: '#f7df1e' },
  jsx: { icon: 'i-carbon-code', color: '#f7df1e' },
  // Python
  py: { icon: 'i-carbon-code', color: '#3776ab' },
  // JSON
  json: { icon: 'i-carbon-json', color: '#cbcb41' },
  // Markdown
  md: { icon: 'i-carbon-document', color: '#519aba' },
  // HTML
  html: { icon: 'i-carbon-html', color: '#e34c26' },
  htm: { icon: 'i-carbon-html', color: '#e34c26' },
  // CSS
  css: { icon: 'i-carbon-css', color: '#264de4' },
  scss: { icon: 'i-carbon-css', color: '#c6538c' },
  sass: { icon: 'i-carbon-css', color: '#c6538c' },
  less: { icon: 'i-carbon-css', color: '#1d365d' },
  // YAML/TOML
  yaml: { icon: 'i-carbon-document', color: '#cb171e' },
  yml: { icon: 'i-carbon-document', color: '#cb171e' },
  toml: { icon: 'i-carbon-document', color: '#9c4121' },
  // XML
  xml: { icon: 'i-carbon-document', color: '#e37933' },
  // SQL
  sql: { icon: 'i-carbon-data-base', color: '#336791' },
  // Shell
  sh: { icon: 'i-carbon-terminal', color: '#89e051' },
  bash: { icon: 'i-carbon-terminal', color: '#89e051' },
  // Go
  go: { icon: 'i-carbon-code', color: '#00add8' },
  // Java
  java: { icon: 'i-carbon-code', color: '#b07219' },
  // C/C++
  c: { icon: 'i-carbon-code', color: '#555555' },
  cpp: { icon: 'i-carbon-code', color: '#f34b7d' },
  h: { icon: 'i-carbon-code', color: '#555555' },
  hpp: { icon: 'i-carbon-code', color: '#f34b7d' },
  // C#
  cs: { icon: 'i-carbon-code', color: '#178600' },
  // Ruby
  rb: { icon: 'i-carbon-code', color: '#701516' },
  // PHP
  php: { icon: 'i-carbon-code', color: '#4f5d95' },
  // 文本文件
  txt: { icon: 'i-carbon-document-blank', color: '#6b7280' },
}

// 默认文件图标
const DEFAULT_FILE_ICON: FileIconConfig = {
  icon: 'i-carbon-document-blank',
  color: '#6b7280',
}

// 目录图标配置
const DIRECTORY_ICON: FileIconConfig = {
  icon: 'i-carbon-folder',
  color: '#14b8a6',
}

// 获取文件图标配置
function getFileIconConfig(fileName: string, isDirectory: boolean): FileIconConfig {
  if (isDirectory) {
    return DIRECTORY_ICON
  }
  const ext = fileName.split('.').pop()?.toLowerCase() || ''
  return FILE_ICON_MAP[ext] || DEFAULT_FILE_ICON
}

// 根据后端返回的文件列表构建简单树结构
const treeData = computed<IndexTreeNode[]>(() => {
  const result: IndexTreeNode[] = []
  const allFiles = filesStatus.value?.files ?? []

  // 根据开关过滤文件列表：仅保留状态不是 indexed 的文件
  const files = showOnlyPending.value
    ? allFiles.filter(file => file.status !== 'indexed')
    : allFiles

  for (const file of files) {
    insertPath(result, file.path, file.status)
  }

  // 构建完成后，为目录节点计算聚合状态并更新标签文案
  aggregateDirectoryStats(result)

  return result
})

// 将单个文件路径插入到树结构中
function insertPath(nodes: IndexTreeNode[], path: string, status: FileIndexStatusType) {
  // 只区分 indexed / pending 两种状态，mixed 由前端文案解释
  const normalizedStatus: NodeStatus = status === 'indexed' ? 'indexed' : 'pending'

  const segments = path.split('/').filter(Boolean)
  let current = nodes
  let currentPath = ''

  segments.forEach((segment, index) => {
    currentPath = currentPath ? `${currentPath}/${segment}` : segment
    let node = current.find(n => n.key === currentPath)

    const isLeaf = index === segments.length - 1

    if (!node) {
      // 提取文件扩展名
      const ext = segment.includes('.') ? segment.split('.').pop()?.toLowerCase() : undefined

      node = {
        key: currentPath,
        label: segment,
        fileName: segment,
        isDirectory: !isLeaf,
        fileExtension: isLeaf ? ext : undefined,
      }
      current.push(node)
    }

    if (isLeaf) {
      // 文件节点：保存原始文件名和状态
      node.status = normalizedStatus
      node.isDirectory = false
    }
    else {
      // 目录节点
      node.isDirectory = true
      if (!node.children)
        node.children = []
      current = node.children
    }
  })
}

// 计算目录节点的聚合状态，并更新目录标签（显示已索引/总文件数）
function aggregateDirectoryStats(nodes: IndexTreeNode[]) {
  nodes.forEach((node) => {
    aggregateNode(node)
  })
}

function aggregateNode(node: IndexTreeNode): { total: number, indexed: number } {
  if (!node.children || node.children.length === 0) {
    const total = node.status ? 1 : 0
    const indexed = node.status === 'indexed' ? 1 : 0
    return { total, indexed }
  }

  let total = 0
  let indexed = 0

  for (const child of node.children) {
    const childAgg = aggregateNode(child)
    total += childAgg.total
    indexed += childAgg.indexed
  }

  if (total > 0) {
    const baseLabel = node.label.split(' · ')[0]
    let suffix: string

    if (indexed === 0) {
      suffix = '未索引'
    }
    else if (indexed === total) {
      suffix = `${indexed}`
    }
    else {
      suffix = `${indexed}/${total}`
    }

    node.label = `${baseLabel} · ${suffix}`
  }

  return { total, indexed }
}

// 加载指定项目的文件索引状态
async function fetchFilesStatus() {
  if (!props.projectRoot)
    return

  loadingFiles.value = true
  filesError.value = null

  try {
    // 调用 Tauri 命令获取文件级索引状态
    const result = await invoke<ProjectFilesStatus>('get_acemcp_project_files_status', {
      projectRootPath: props.projectRoot,
    })
    filesStatus.value = result
  }
  catch (err) {
    console.error('获取项目文件索引状态失败:', err)
    filesError.value = String(err)
    message.error('加载项目结构失败，请检查索引配置')
  }
  finally {
    loadingFiles.value = false
  }
}

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

// 是否有嵌套项目
const hasNestedProjects = computed(() => {
  return (nestedStatus.value?.nested_projects?.length ?? 0) > 0
})

// 嵌套项目列表
const nestedProjects = computed(() => nestedStatus.value?.nested_projects ?? [])

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

// 当弹窗打开或项目路径变化时，刷新文件状态与嵌套项目状态
watch(
  () => [props.show, props.projectRoot],
  ([visible, root]) => {
    if (visible && root) {
      // 重置状态，避免切换项目时显示旧数据
      filesStatus.value = null
      nestedStatus.value = null
      filesError.value = null
      nestedError.value = null
      fetchFilesStatus()
      fetchNestedStatus()
    }
  },
)

// 手动重新同步按钮点击
function handleResyncClick() {
  emit('resync')
}

// ==================== 自定义树节点渲染 ====================

// 渲染节点前缀图标
function renderPrefix({ option }: { option: TreeOption }) {
  const node = option as unknown as IndexTreeNode
  const iconConfig = getFileIconConfig(node.fileName || node.label, node.isDirectory || false)

  return h('div', {
    class: `${iconConfig.icon} w-3.5 h-3.5 flex-shrink-0`,
    style: { color: iconConfig.color },
  })
}

// 渲染节点标签
function renderLabel({ option }: { option: TreeOption }) {
  const node = option as unknown as IndexTreeNode
  const isDirectory = node.isDirectory || false
  const fileName = node.fileName || node.label.split(' · ')[0]

  // 目录节点：显示目录名和统计信息
  if (isDirectory) {
    const stats = node.label.includes(' · ') ? node.label.split(' · ')[1] : ''
    return h('div', { class: 'flex items-center gap-1.5' }, [
      h('span', { class: 'tree-node-label truncate' }, fileName),
      stats
        ? h('span', {
            class: 'tree-node-badge tree-node-badge--stats',
          }, stats)
        : null,
    ])
  }

  // 文件节点：显示文件名和状态图标
  const status = node.status
  return h('div', { class: 'flex items-center gap-1.5' }, [
    h('span', { class: 'tree-node-label tree-node-label--file truncate' }, fileName),
    // 状态图标：已索引（绿色勾选）/ 未同步（橙色圆圈）
    h('div', {
      class: status === 'indexed'
        ? 'i-carbon-checkmark-filled w-3 h-3 text-green-500'
        : 'i-carbon-circle-dash w-3 h-3 text-orange-500',
    }),
  ])
}

// 复制路径到剪贴板
function handleCopyPath() {
  if (displayPath.value) {
    navigator.clipboard.writeText(displayPath.value)
    message.success('路径已复制')
  }
}
</script>

<template>
  <n-modal
    v-model:show="modalVisible"
    preset="card"
    class="index-status-modal"
    :style="{ width: '680px', maxHeight: '70vh' }"
    :bordered="false"
    :closable="true"
    :mask-closable="true"
    :segmented="{ content: true, footer: 'soft' }"
  >
    <!-- 头部标题 -->
    <template #header>
      <div class="modal-header">
        <div class="header-icon" :class="statusIcon" />
        <span class="header-title">代码索引状态</span>
        <n-tag
          size="small"
          :type="projectStatus?.status === 'synced' ? 'success' : projectStatus?.status === 'failed' ? 'error' : 'info'"
        >
          {{ statusSummary }}
        </n-tag>
      </div>
    </template>

    <!-- 主内容区域：左右双栏布局 -->
    <div class="modal-content">
      <!-- 左侧：统计信息 -->
      <div class="stats-panel">
        <!-- 项目信息卡片 -->
        <div class="info-card">
          <div class="info-card__header">
            <div class="i-carbon-folder text-primary-500" />
            <span class="info-card__title">{{ projectName }}</span>
          </div>
          <div
            class="info-card__path"
            :title="displayPath"
            @click="handleCopyPath"
          >
            {{ displayPath || '未指定路径' }}
          </div>
        </div>

        <!-- 进度展示 -->
        <div v-if="projectStatus" class="progress-card">
          <div class="progress-card__header">
            <span class="progress-card__label">索引进度</span>
            <span class="progress-card__value">{{ projectStatus.progress }}%</span>
          </div>
          <n-progress
            type="line"
            :percentage="projectStatus.progress"
            :height="6"
            :border-radius="3"
            :show-indicator="false"
            :status="projectStatus.status === 'failed' ? 'error' : projectStatus.progress === 100 ? 'success' : 'info'"
            processing
          />
        </div>

        <!-- 文件统计 -->
        <div class="stats-grid">
          <div class="stat-item">
            <div class="i-carbon-document stat-item__icon" />
            <div class="stat-item__content">
              <span class="stat-item__value">{{ projectStatus?.total_files ?? 0 }}</span>
              <span class="stat-item__label">总文件</span>
            </div>
          </div>
          <div class="stat-item stat-item--success">
            <div class="i-carbon-checkmark-filled stat-item__icon" />
            <div class="stat-item__content">
              <span class="stat-item__value">{{ projectStatus?.indexed_files ?? 0 }}</span>
              <span class="stat-item__label">已索引</span>
            </div>
          </div>
          <div v-if="(projectStatus?.pending_files ?? 0) > 0" class="stat-item stat-item--info">
            <div class="i-carbon-time stat-item__icon" />
            <div class="stat-item__content">
              <span class="stat-item__value">{{ projectStatus?.pending_files ?? 0 }}</span>
              <span class="stat-item__label">待处理</span>
            </div>
          </div>
          <div v-if="(projectStatus?.failed_files ?? 0) > 0" class="stat-item stat-item--error">
            <div class="i-carbon-warning-filled stat-item__icon" />
            <div class="stat-item__content">
              <span class="stat-item__value">{{ projectStatus?.failed_files ?? 0 }}</span>
              <span class="stat-item__label">失败</span>
            </div>
          </div>
        </div>

        <!-- 时间信息 -->
        <div v-if="projectStatus?.last_success_time" class="time-info">
          <div class="i-carbon-time" />
          <span>上次成功：{{ projectStatus.last_success_time }}</span>
        </div>

        <!-- 错误信息 -->
        <div v-if="projectStatus?.last_error" class="error-info">
          <div class="i-carbon-warning-alt" />
          <span>{{ projectStatus.last_error }}</span>
        </div>

        <!-- 嵌套项目区域（如果有） -->
        <div v-if="hasNestedProjects || nestedError" class="nested-projects-card">
          <div class="nested-card__header">
            <div class="i-carbon-folder-parent nested-card__icon" />
            <span class="nested-card__title">Git 子项目</span>
            <span class="nested-card__count">{{ nestedProjects.length }}</span>
          </div>
          <!-- 骨架屏 -->
          <div v-if="loadingNested" class="nested-skeleton">
            <div v-for="i in 3" :key="i" class="skeleton-row">
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
          <div v-else class="nested-card__list">
            <div
              v-for="np in nestedProjects"
              :key="np.absolute_path"
              class="nested-card__item"
            >
              <div class="nested-card__item-left">
                <div class="i-carbon-folder-details nested-card__folder-icon" />
                <span class="nested-card__item-name">{{ np.relative_path }}</span>
              </div>
              <div class="nested-card__item-right">
                <span class="nested-card__item-stats">{{ getNestedStatusText(np) }}</span>
                <div :class="getNestedStatusIcon(np)" class="nested-card__item-status" />
              </div>
            </div>
          </div>
        </div>
      </div>

      <!-- 右侧：文件树 -->
      <div class="tree-panel">
        <!-- 工具栏 -->
        <div class="tree-toolbar">
          <div class="tree-toolbar__left">
            <span class="tree-toolbar__title">项目结构</span>
            <n-switch
              v-model:value="showOnlyPending"
              size="small"
            />
            <span class="tree-toolbar__filter-label">仅未同步</span>
          </div>
          <n-button
            text
            size="tiny"
            :loading="loadingFiles"
            @click="fetchFilesStatus"
          >
            <template #icon>
              <div class="i-carbon-renew w-3.5 h-3.5" />
            </template>
          </n-button>
        </div>

        <!-- 树容器 -->
        <div class="tree-container">
          <!-- 骨架屏加载状态 -->
          <div v-if="loadingFiles" class="tree-skeleton">
            <div v-for="i in 8" :key="i" class="skeleton-row" :style="{ paddingLeft: `${(i % 4) * 12}px` }">
              <div class="skeleton-icon" />
              <div class="skeleton-text" :style="{ width: `${50 + Math.random() * 80}px` }" />
            </div>
          </div>

          <!-- 错误状态 -->
          <div v-else-if="filesError" class="tree-error">
            <div class="i-carbon-warning-alt" />
            <span>{{ filesError }}</span>
          </div>

          <!-- 空状态 -->
          <div v-else-if="!treeData.length" class="tree-empty">
            <div class="i-carbon-folder-off" />
            <span>暂无可索引文件</span>
            <span class="tree-empty__hint">请确认扩展名和排除规则配置</span>
          </div>

          <!-- 项目结构树 -->
          <div v-else class="tree-wrapper">
            <n-tree
              :data="treeData"
              :block-line="true"
              :selectable="false"
              :expand-on-click="true"
              :render-prefix="renderPrefix"
              :render-label="renderLabel"
              :default-expand-all="false"
              :animated="true"
            />
          </div>
        </div>
      </div>
    </div>

    <!-- 底部操作区 -->
    <template #footer>
      <div class="modal-footer">
        <div class="footer-hint">
          重新同步会在后台执行，不会阻塞当前对话
        </div>
        <n-button
          type="primary"
          size="small"
          :loading="resyncLoading || isIndexing"
          :disabled="resyncLoading || isIndexing || !projectRoot"
          @click="handleResyncClick"
        >
          <template #icon>
            <div class="i-carbon-renew" />
          </template>
          {{ isIndexing ? '索引中...' : '重新同步' }}
        </n-button>
      </div>
    </template>
  </n-modal>
</template>

<style scoped>
/* ==================== 弹窗整体样式 ==================== */
.index-status-modal :deep(.n-card) {
  border-radius: 12px;
  overflow: hidden;
}

.index-status-modal :deep(.n-card__content) {
  padding: 0 !important;
}

/* ==================== 头部样式 ==================== */
.modal-header {
  display: flex;
  align-items: center;
  gap: 10px;
}

.header-icon {
  width: 18px;
  height: 18px;
}

.header-title {
  font-size: 15px;
  font-weight: 600;
  color: var(--color-on-surface, #111827);
}

:root.dark .header-title {
  color: #e5e7eb;
}

/* ==================== 主内容区域 ==================== */
.modal-content {
  display: flex;
  gap: 16px;
  padding: 16px;
  min-height: 280px;
  max-height: calc(70vh - 120px);
}

/* ==================== 左侧统计面板 ==================== */
.stats-panel {
  width: 200px;
  flex-shrink: 0;
  display: flex;
  flex-direction: column;
  gap: 12px;
}

/* 项目信息卡片 */
.info-card {
  padding: 12px;
  border-radius: 8px;
  background: var(--color-container, rgba(0, 0, 0, 0.03));
  border: 1px solid var(--color-border, rgba(128, 128, 128, 0.15));
}

:root.dark .info-card {
  background: rgba(255, 255, 255, 0.03);
  border-color: rgba(255, 255, 255, 0.08);
}

.info-card__header {
  display: flex;
  align-items: center;
  gap: 6px;
  margin-bottom: 6px;
}

.info-card__title {
  font-size: 13px;
  font-weight: 600;
  color: var(--color-on-surface, #111827);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

:root.dark .info-card__title {
  color: #e5e7eb;
}

.info-card__path {
  font-size: 10px;
  font-family: ui-monospace, monospace;
  color: var(--color-on-surface-secondary, #6b7280);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  cursor: pointer;
  transition: color 0.2s;
}

.info-card__path:hover {
  color: var(--color-primary, #14b8a6);
}

:root.dark .info-card__path {
  color: #9ca3af;
}

/* 进度卡片 */
.progress-card {
  padding: 10px 12px;
  border-radius: 8px;
  background: var(--color-container, rgba(0, 0, 0, 0.03));
  border: 1px solid var(--color-border, rgba(128, 128, 128, 0.15));
}

:root.dark .progress-card {
  background: rgba(255, 255, 255, 0.03);
  border-color: rgba(255, 255, 255, 0.08);
}

.progress-card__header {
  display: flex;
  justify-content: space-between;
  align-items: center;
  margin-bottom: 8px;
}

.progress-card__label {
  font-size: 11px;
  color: var(--color-on-surface-secondary, #6b7280);
}

:root.dark .progress-card__label {
  color: #9ca3af;
}

.progress-card__value {
  font-size: 13px;
  font-weight: 600;
  color: var(--color-primary, #14b8a6);
}

/* 统计网格 */
.stats-grid {
  display: grid;
  grid-template-columns: repeat(2, 1fr);
  gap: 8px;
}

.stat-item {
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 8px 10px;
  border-radius: 6px;
  background: var(--color-container, rgba(0, 0, 0, 0.03));
  border: 1px solid var(--color-border, rgba(128, 128, 128, 0.15));
}

:root.dark .stat-item {
  background: rgba(255, 255, 255, 0.03);
  border-color: rgba(255, 255, 255, 0.08);
}

.stat-item__icon {
  width: 14px;
  height: 14px;
  color: var(--color-on-surface-secondary, #6b7280);
}

:root.dark .stat-item__icon {
  color: #9ca3af;
}

.stat-item--success .stat-item__icon { color: #22c55e; }
.stat-item--info .stat-item__icon { color: #3b82f6; }
.stat-item--error .stat-item__icon { color: #ef4444; }

.stat-item__content {
  display: flex;
  flex-direction: column;
  min-width: 0;
}

.stat-item__value {
  font-size: 14px;
  font-weight: 600;
  line-height: 1.2;
  color: var(--color-on-surface, #111827);
}

:root.dark .stat-item__value {
  color: #e5e7eb;
}

.stat-item__label {
  font-size: 9px;
  color: var(--color-on-surface-secondary, #6b7280);
}

:root.dark .stat-item__label {
  color: #9ca3af;
}

/* 时间信息 */
.time-info {
  display: flex;
  align-items: center;
  gap: 6px;
  font-size: 10px;
  color: var(--color-on-surface-secondary, #6b7280);
}

:root.dark .time-info {
  color: #9ca3af;
}

/* 错误信息 */
.error-info {
  display: flex;
  align-items: flex-start;
  gap: 6px;
  padding: 8px;
  border-radius: 6px;
  font-size: 10px;
  color: #ef4444;
  background: rgba(239, 68, 68, 0.1);
  border: 1px solid rgba(239, 68, 68, 0.2);
}

/* ==================== 右侧文件树面板 ==================== */
.tree-panel {
  flex: 1;
  min-width: 0;
  display: flex;
  flex-direction: column;
  border-radius: 8px;
  background: var(--color-container, rgba(0, 0, 0, 0.03));
  border: 1px solid var(--color-border, rgba(128, 128, 128, 0.15));
  overflow: hidden;
}

:root.dark .tree-panel {
  background: rgba(255, 255, 255, 0.03);
  border-color: rgba(255, 255, 255, 0.08);
}

/* 工具栏 */
.tree-toolbar {
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: 8px 12px;
  border-bottom: 1px solid var(--color-border, rgba(128, 128, 128, 0.15));
  background: var(--color-container, rgba(0, 0, 0, 0.02));
}

:root.dark .tree-toolbar {
  border-color: rgba(255, 255, 255, 0.08);
  background: rgba(255, 255, 255, 0.02);
}

.tree-toolbar__left {
  display: flex;
  align-items: center;
  gap: 8px;
}

.tree-toolbar__title {
  font-size: 11px;
  font-weight: 600;
  color: var(--color-on-surface-secondary, #6b7280);
}

:root.dark .tree-toolbar__title {
  color: #9ca3af;
}

.tree-toolbar__filter-label {
  font-size: 10px;
  color: var(--color-on-surface-secondary, #6b7280);
}

:root.dark .tree-toolbar__filter-label {
  color: #9ca3af;
}

/* 树容器 */
.tree-container {
  flex: 1;
  overflow-y: auto;
  padding: 8px;
}

/* 骨架屏 */
.tree-skeleton {
  display: flex;
  flex-direction: column;
  gap: 6px;
  padding: 4px;
}

.skeleton-row {
  display: flex;
  align-items: center;
  gap: 8px;
  animation: skeleton-pulse 1.5s ease-in-out infinite;
}

.skeleton-icon {
  width: 14px;
  height: 14px;
  border-radius: 3px;
  background: linear-gradient(90deg, rgba(128, 128, 128, 0.1) 25%, rgba(128, 128, 128, 0.2) 50%, rgba(128, 128, 128, 0.1) 75%);
  background-size: 200% 100%;
  animation: skeleton-shimmer 1.5s ease-in-out infinite;
}

.skeleton-text {
  height: 12px;
  border-radius: 3px;
  background: linear-gradient(90deg, rgba(128, 128, 128, 0.1) 25%, rgba(128, 128, 128, 0.2) 50%, rgba(128, 128, 128, 0.1) 75%);
  background-size: 200% 100%;
  animation: skeleton-shimmer 1.5s ease-in-out infinite;
}

@keyframes skeleton-shimmer {
  0% { background-position: 200% 0; }
  100% { background-position: -200% 0; }
}

@keyframes skeleton-pulse {
  0%, 100% { opacity: 0.6; }
  50% { opacity: 1; }
}

/* 错误状态 */
.tree-error {
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 12px;
  color: #ef4444;
  font-size: 12px;
}

/* 空状态 */
.tree-empty {
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  padding: 32px 16px;
  color: var(--color-on-surface-secondary, #6b7280);
}

:root.dark .tree-empty {
  color: #9ca3af;
}

.tree-empty > div:first-child {
  width: 32px;
  height: 32px;
  margin-bottom: 8px;
  opacity: 0.5;
}

.tree-empty > span:first-of-type {
  font-size: 12px;
  font-weight: 500;
}

.tree-empty__hint {
  font-size: 10px;
  opacity: 0.7;
  margin-top: 4px;
}

/* 树包装器 */
.tree-wrapper {
  font-size: 12px;
}

/* 树节点样式 */
.tree-wrapper :deep(.n-tree-node) {
  border-radius: 4px;
  margin-bottom: 1px;
  padding: 0 4px;
}

.tree-wrapper :deep(.n-tree-node:hover) {
  background: rgba(128, 128, 128, 0.08);
}

.tree-wrapper :deep(.n-tree-node-content) {
  padding: 2px 4px;
}

.tree-wrapper :deep(.n-tree-node-switcher) {
  width: 16px;
  height: 16px;
}

/* 节点标签 */
.tree-node-label {
  font-size: 11px;
  color: var(--color-on-surface, #111827);
}

:root.dark .tree-node-label {
  color: #e5e7eb;
}

.tree-node-label--file {
  color: var(--color-on-surface-secondary, #6b7280);
}

:root.dark .tree-node-label--file {
  color: #d1d5db;
}

/* 节点徽章 */
.tree-node-badge {
  font-size: 9px;
  padding: 1px 4px;
  border-radius: 3px;
  font-weight: 500;
}

.tree-node-badge--stats {
  background: rgba(20, 184, 166, 0.15);
  color: #14b8a6;
}

.tree-node-badge--indexed {
  color: #22c55e;
}

.tree-node-badge--pending {
  color: #f59e0b;
}

/* ==================== 底部操作栏 ==================== */
.modal-footer {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 16px;
}

.footer-hint {
  font-size: 11px;
  color: var(--color-on-surface-secondary, #6b7280);
}

:root.dark .footer-hint {
  color: #9ca3af;
}

/* ==================== 嵌套项目卡片 ==================== */
.nested-projects-card {
  margin-top: 12px;
  padding: 12px;
  border-radius: 10px;
  background: linear-gradient(135deg, rgba(52, 211, 153, 0.06) 0%, rgba(52, 211, 153, 0.02) 100%);
  border: 1px solid rgba(52, 211, 153, 0.12);
}

.nested-card__header {
  display: flex;
  align-items: center;
  gap: 8px;
  margin-bottom: 10px;
}

.nested-card__icon {
  width: 16px;
  height: 16px;
  color: rgba(52, 211, 153, 0.8);
}

.nested-card__title {
  font-size: 12px;
  font-weight: 600;
  color: rgba(52, 211, 153, 0.95);
  text-transform: uppercase;
  letter-spacing: 0.5px;
}

.nested-card__count {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  min-width: 18px;
  height: 18px;
  padding: 0 5px;
  border-radius: 9px;
  font-size: 10px;
  font-weight: 600;
  background: rgba(52, 211, 153, 0.2);
  color: rgba(52, 211, 153, 0.95);
}

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

.nested-card__list {
  display: flex;
  flex-direction: column;
  gap: 6px;
}

.nested-card__item {
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: 8px 10px;
  border-radius: 8px;
  background: rgba(255, 255, 255, 0.03);
  transition: all 0.2s ease;
}

.nested-card__item:hover {
  background: rgba(255, 255, 255, 0.06);
}

.nested-card__item-left {
  display: flex;
  align-items: center;
  gap: 8px;
}

.nested-card__folder-icon {
  width: 14px;
  height: 14px;
  color: rgba(52, 211, 153, 0.6);
}

.nested-card__item-name {
  font-size: 12px;
  font-weight: 500;
  color: rgba(255, 255, 255, 0.9);
}

.nested-card__item-right {
  display: flex;
  align-items: center;
  gap: 8px;
}

.nested-card__item-stats {
  font-size: 11px;
  color: rgba(255, 255, 255, 0.5);
  font-family: ui-monospace, SFMono-Regular, monospace;
}

.nested-card__item-status {
  width: 12px;
  height: 12px;
}

/* 骨架屏 */
.nested-skeleton {
  display: flex;
  flex-direction: column;
  gap: 8px;
}

.skeleton-row {
  display: flex;
  align-items: center;
  gap: 10px;
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
  width: 100px;
  border-radius: 4px;
  background: rgba(255, 255, 255, 0.08);
  animation: skeleton-pulse 1.5s ease-in-out infinite;
}

@keyframes skeleton-pulse {
  0%, 100% { opacity: 1; }
  50% { opacity: 0.4; }
}
</style>
