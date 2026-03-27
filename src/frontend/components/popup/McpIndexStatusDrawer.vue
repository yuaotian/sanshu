<script setup lang="ts">
import { invoke } from '@tauri-apps/api/core'
import type { TreeOption } from 'naive-ui'
import { NTag, useMessage } from 'naive-ui'
import { computed, h, ref, watch } from 'vue'
import type { FileIndexStatusType, NestedProjectInfo, ProjectFilesStatus, ProjectIndexStatus, ProjectWithNestedStatus } from '../../types/tauri'
import AppModal from '../common/AppModal.vue'

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

const isStaleProject = computed(() => {
  return !!props.projectStatus?.is_stale && props.projectStatus?.status !== 'indexing'
})

const displayStatusSummary = computed(() => {
  return isStaleProject.value ? '配置已变更' : props.statusSummary
})

const displayStatusIcon = computed(() => {
  return isStaleProject.value ? 'i-carbon-warning-alt text-warning' : props.statusIcon
})

const displayStatusTagType = computed(() => {
  if (isStaleProject.value)
    return 'warning'
  return props.projectStatus?.status === 'synced'
    ? 'success'
    : props.projectStatus?.status === 'failed'
      ? 'error'
      : 'info'
})

const staleMessage = computed(() => {
  if (!isStaleProject.value)
    return ''
  return props.projectStatus?.stale_reason || '检测到 ACE 配置已变更，旧索引等待重新索引'
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

interface FileIconConfig {
  icon: string
  colorClass: string
}

const FILE_ICON_MAP: Record<string, FileIconConfig> = {
  rs: { icon: 'i-carbon-code', colorClass: 'text-primary' },
  vue: { icon: 'i-carbon-application', colorClass: 'text-primary' },
  ts: { icon: 'i-carbon-code', colorClass: 'text-primary' },
  tsx: { icon: 'i-carbon-code', colorClass: 'text-primary' },
  js: { icon: 'i-carbon-code', colorClass: 'text-primary' },
  jsx: { icon: 'i-carbon-code', colorClass: 'text-primary' },
  py: { icon: 'i-carbon-code', colorClass: 'text-primary' },
  json: { icon: 'i-carbon-json', colorClass: 'text-info' },
  md: { icon: 'i-carbon-document', colorClass: 'text-on-surface-secondary' },
  html: { icon: 'i-carbon-html', colorClass: 'text-warning' },
  htm: { icon: 'i-carbon-html', colorClass: 'text-warning' },
  css: { icon: 'i-carbon-css', colorClass: 'text-info' },
  scss: { icon: 'i-carbon-css', colorClass: 'text-info' },
  sass: { icon: 'i-carbon-css', colorClass: 'text-info' },
  less: { icon: 'i-carbon-css', colorClass: 'text-info' },
  yaml: { icon: 'i-carbon-document', colorClass: 'text-info' },
  yml: { icon: 'i-carbon-document', colorClass: 'text-info' },
  toml: { icon: 'i-carbon-document', colorClass: 'text-info' },
  xml: { icon: 'i-carbon-document', colorClass: 'text-info' },
  sql: { icon: 'i-carbon-data-base', colorClass: 'text-info' },
  sh: { icon: 'i-carbon-terminal', colorClass: 'text-success' },
  bash: { icon: 'i-carbon-terminal', colorClass: 'text-success' },
  go: { icon: 'i-carbon-code', colorClass: 'text-primary' },
  java: { icon: 'i-carbon-code', colorClass: 'text-primary' },
  c: { icon: 'i-carbon-code', colorClass: 'text-primary' },
  cpp: { icon: 'i-carbon-code', colorClass: 'text-primary' },
  h: { icon: 'i-carbon-code', colorClass: 'text-primary' },
  hpp: { icon: 'i-carbon-code', colorClass: 'text-primary' },
  cs: { icon: 'i-carbon-code', colorClass: 'text-primary' },
  rb: { icon: 'i-carbon-code', colorClass: 'text-primary' },
  php: { icon: 'i-carbon-code', colorClass: 'text-primary' },
  txt: { icon: 'i-carbon-document-blank', colorClass: 'text-on-surface-muted' },
}

const DEFAULT_FILE_ICON: FileIconConfig = {
  icon: 'i-carbon-document-blank',
  colorClass: 'text-on-surface-muted',
}

const DIRECTORY_ICON: FileIconConfig = {
  icon: 'i-carbon-folder',
  colorClass: 'text-info',
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
  if (np.index_status?.is_stale && np.index_status.status !== 'indexing')
    return 'i-carbon-warning-alt text-warning'

  const status = np.index_status?.status
  switch (status) {
    case 'synced':
      return 'i-carbon-checkmark-filled text-success'
    case 'indexing':
      return 'i-carbon-in-progress text-success opacity-80 animate-spin'
    case 'failed':
      return 'i-carbon-warning-filled text-error'
    default:
      return 'i-carbon-circle-dash text-on-surface-muted opacity-60'
  }
}

// 获取子项目状态文字
function getNestedStatusText(np: NestedProjectInfo): string {
  const status = np.index_status
  if (!status)
    return '未索引'
  if (status.is_stale && status.status !== 'indexing')
    return '待重建'
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
    class: `${iconConfig.icon} w-3.5 h-3.5 flex-shrink-0 ${iconConfig.colorClass}`,
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
      h('span', { class: 'text-[11px] text-on-surface truncate' }, fileName),
      stats
        ? h(NTag, { size: 'tiny', type: 'primary', bordered: false, round: true }, { default: () => stats })
        : null,
    ])
  }

  const status = node.status
  return h('div', { class: 'flex items-center gap-1.5' }, [
    h('span', { class: 'text-[11px] text-on-surface-secondary truncate' }, fileName),
    h('div', {
      class: status === 'indexed'
        ? 'i-carbon-checkmark-filled w-3 h-3 text-success'
        : 'i-carbon-circle-dash w-3 h-3 text-warning',
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
  <AppModal
    v-model:show="modalVisible"
    width="760px"
    max-height="70vh"
    body-overflow="hidden"
    :segmented="{ content: true, footer: 'soft' }"
  >
    <template #header>
      <n-space align="center" :size="10">
        <div class="h-4.5 w-4.5 shrink-0" :class="displayStatusIcon" />
        <n-text strong class="text-[15px]">
          代码索引状态
        </n-text>
        <n-tag
          size="small"
          :type="displayStatusTagType"
        >
          {{ displayStatusSummary }}
        </n-tag>
      </n-space>
    </template>

    <div class="flex gap-4 min-h-[280px] max-h-[calc(70vh-120px)] overflow-hidden">
      <div class="w-[200px] shrink-0 flex flex-col gap-3 overflow-y-auto">
        <n-card size="small" :bordered="true">
          <n-space vertical :size="6">
            <n-space align="center" :size="6" :wrap="false">
              <div class="i-carbon-folder h-4 w-4 shrink-0 text-primary" />
              <n-text strong class="truncate min-w-0 text-[13px]">
                {{ projectName }}
              </n-text>
            </n-space>
            <n-text
              depth="3"
              class="text-[10px] font-mono truncate cursor-pointer hover:text-primary"
              :title="displayPath"
              @click="handleCopyPath"
            >
              {{ displayPath || '未指定路径' }}
            </n-text>
          </n-space>
        </n-card>

        <n-card v-if="projectStatus" size="small" :bordered="true">
          <n-space vertical :size="8">
            <n-space justify="space-between" align="center">
              <n-text depth="3" class="text-[11px]">
                索引进度
              </n-text>
              <n-text type="primary" strong class="text-[13px]">
                {{ projectStatus.progress }}%
              </n-text>
            </n-space>
            <n-progress
              type="line"
              :percentage="projectStatus.progress"
              :height="6"
              :border-radius="3"
              :show-indicator="false"
              :status="projectStatus.status === 'failed' ? 'error' : projectStatus.progress === 100 ? 'success' : 'info'"
              processing
            />
          </n-space>
        </n-card>

        <div class="grid grid-cols-2 gap-2">
          <n-card size="small" :bordered="true">
            <n-space align="center" :size="8">
              <div class="i-carbon-document h-3.5 w-3.5 shrink-0 text-on-surface-secondary" />
              <n-space vertical :size="0">
                <n-text strong class="text-sm leading-tight">
                  {{ projectStatus?.total_files ?? 0 }}
                </n-text>
                <n-text depth="3" class="text-[9px] leading-tight">
                  总文件
                </n-text>
              </n-space>
            </n-space>
          </n-card>
          <n-card size="small" :bordered="true">
            <n-space align="center" :size="8">
              <div class="i-carbon-checkmark-filled h-3.5 w-3.5 shrink-0 text-success" />
              <n-space vertical :size="0">
                <n-text strong class="text-sm leading-tight">
                  {{ projectStatus?.indexed_files ?? 0 }}
                </n-text>
                <n-text depth="3" class="text-[9px] leading-tight">
                  已索引
                </n-text>
              </n-space>
            </n-space>
          </n-card>
          <n-card v-if="(projectStatus?.pending_files ?? 0) > 0" size="small" :bordered="true">
            <n-space align="center" :size="8">
              <div class="i-carbon-time h-3.5 w-3.5 shrink-0 text-info" />
              <n-space vertical :size="0">
                <n-text strong class="text-sm leading-tight">
                  {{ projectStatus?.pending_files ?? 0 }}
                </n-text>
                <n-text depth="3" class="text-[9px] leading-tight">
                  待处理
                </n-text>
              </n-space>
            </n-space>
          </n-card>
          <n-card v-if="(projectStatus?.failed_files ?? 0) > 0" size="small" :bordered="true">
            <n-space align="center" :size="8">
              <div class="i-carbon-warning-filled h-3.5 w-3.5 shrink-0 text-error" />
              <n-space vertical :size="0">
                <n-text strong class="text-sm leading-tight">
                  {{ projectStatus?.failed_files ?? 0 }}
                </n-text>
                <n-text depth="3" class="text-[9px] leading-tight">
                  失败
                </n-text>
              </n-space>
            </n-space>
          </n-card>
        </div>

        <n-space v-if="projectStatus?.last_success_time" align="center" :size="6">
          <div class="i-carbon-time shrink-0 text-on-surface-secondary" />
          <n-text depth="3" class="text-[10px] text-on-surface-secondary">
            上次成功：{{ projectStatus.last_success_time }}
          </n-text>
        </n-space>

        <n-alert v-if="staleMessage" type="warning" size="small" :bordered="false">
          {{ staleMessage }}
        </n-alert>

        <n-alert v-if="projectStatus?.last_error" type="error" size="small" :bordered="false">
          {{ projectStatus.last_error }}
        </n-alert>

        <n-card v-if="hasNestedProjects || nestedError" size="small" :bordered="true">
          <template #header>
            <n-space align="center" :size="8">
              <div class="i-carbon-folder-parent h-4 w-4 shrink-0 text-success" />
              <n-text strong class="text-xs uppercase tracking-wide text-success">
                Git 子项目
              </n-text>
              <n-tag size="small" round type="success" :bordered="false">
                {{ nestedProjects.length }}
              </n-tag>
            </n-space>
          </template>
          <n-space v-if="loadingNested" vertical :size="8">
            <n-space v-for="i in 3" :key="i" align="center" :size="10">
              <n-skeleton height="14px" width="14px" />
              <n-skeleton height="12px" width="100px" />
            </n-space>
          </n-space>
          <n-alert v-else-if="nestedError" type="error" size="small" :bordered="false">
            {{ nestedError }}
          </n-alert>
          <n-space v-else vertical :size="6">
            <n-card
              v-for="np in nestedProjects"
              :key="np.absolute_path"
              size="small"
              embedded
              :bordered="true"
            >
              <n-space justify="space-between" align="center">
                <n-space align="center" :size="8">
                  <div class="i-carbon-folder-details h-3.5 w-3.5 shrink-0 text-success opacity-60" />
                  <n-text class="min-w-0 truncate text-xs font-medium text-on-surface">
                    {{ np.relative_path }}
                  </n-text>
                </n-space>
                <n-space align="center" :size="8">
                  <n-text depth="3" class="text-[11px] font-mono text-on-surface-secondary">
                    {{ getNestedStatusText(np) }}
                  </n-text>
                  <div :class="getNestedStatusIcon(np)" class="h-3 w-3 shrink-0" />
                </n-space>
              </n-space>
            </n-card>
          </n-space>
        </n-card>
      </div>

      <n-card
        size="small"
        :bordered="true"
        class="min-w-0 min-h-0 flex-1 flex flex-col overflow-hidden"
        :header-style="{ padding: '8px 12px' }"
        :content-style="{ flex: '1 1 auto', minHeight: 0, overflowY: 'auto', padding: '8px' }"
      >
        <template #header>
          <n-space justify="space-between" align="center">
            <n-space align="center" :size="8">
              <n-text strong depth="3" class="text-[11px]">
                项目结构
              </n-text>
              <n-switch
                v-model:value="showOnlyPending"
                size="small"
              />
              <n-text depth="3" class="text-[10px]">
                仅未同步
              </n-text>
            </n-space>
            <n-button
              text
              size="tiny"
              :loading="loadingFiles"
              @click="fetchFilesStatus"
            >
              <template #icon>
                <div class="i-carbon-renew h-3.5 w-3.5" />
              </template>
            </n-button>
          </n-space>
        </template>

        <n-space v-if="loadingFiles" vertical :size="6" class="p-1">
          <n-space
            v-for="i in 8"
            :key="i"
            align="center"
            :size="8"
            :style="{ paddingLeft: `${(i % 4) * 12}px` }"
          >
            <n-skeleton height="14px" width="14px" />
            <n-skeleton height="12px" :width="`${50 + ((i * 13) % 80)}px`" />
          </n-space>
        </n-space>

        <n-alert v-else-if="filesError" type="error" size="small" :bordered="false">
          {{ filesError }}
        </n-alert>

        <n-empty
          v-else-if="!treeData.length"
          description="暂无可索引文件"
          class="py-8"
        >
          <template #icon>
            <div class="i-carbon-folder-off h-8 w-8 text-on-surface-muted opacity-50" />
          </template>
          <template #extra>
            <n-text depth="3" class="text-[10px] text-on-surface-secondary">
              请确认扩展名和排除规则配置
            </n-text>
          </template>
        </n-empty>

        <div v-else class="text-xs">
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
      </n-card>
    </div>

    <template #footer>
      <n-space justify="space-between" align="center" class="w-full">
        <n-text depth="3" class="text-[11px] text-on-surface-secondary">
          重新同步会在后台执行，不会阻塞当前对话
        </n-text>
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
      </n-space>
    </template>
  </AppModal>
</template>
