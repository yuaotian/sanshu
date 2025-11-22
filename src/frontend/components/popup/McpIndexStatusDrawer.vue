<script setup lang="ts">
import type { FileIndexStatusType, ProjectFilesStatus, ProjectIndexStatus } from '../../types/tauri'
import { invoke } from '@tauri-apps/api/core'
import { useMessage } from 'naive-ui'
import { computed, ref, watch } from 'vue'

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

// 抽屉显示状态，使用 v-model:show 双向绑定父组件
const drawerVisible = computed({
  get: () => props.show,
  set: (val: boolean) => emit('update:show', val),
})

// 文件索引状态数据
const filesStatus = ref<ProjectFilesStatus | null>(null)
const loadingFiles = ref(false)
const filesError = ref<string | null>(null)

const message = useMessage()

// Tree 节点类型
type NodeStatus = 'indexed' | 'pending'

interface IndexTreeNode {
  key: string
  label: string
  children?: IndexTreeNode[]
  // 仅文件节点使用的状态
  status?: NodeStatus
}

// 根据后端返回的文件列表构建简单树结构
const treeData = computed<IndexTreeNode[]>(() => {
  const result: IndexTreeNode[] = []
  const files = filesStatus.value?.files ?? []

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

    if (!node) {
      node = {
        key: currentPath,
        label: segment,
      }
      current.push(node)
    }

    const isLeaf = index === segments.length - 1

    if (isLeaf) {
      // 文件节点：在标签中直接附加状态文本
      node.label = normalizedStatus === 'indexed'
        ? `${segment} · 已索引`
        : `${segment} · 未完全同步`
      node.status = normalizedStatus
    }
    else {
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
      suffix = `全部已索引 (${indexed})`
    }
    else {
      suffix = `${indexed}/${total} 已索引`
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

// 当抽屉打开且有项目路径时，自动加载一次文件状态
watch(
  () => props.show,
  (visible) => {
    if (visible && props.projectRoot)
      fetchFilesStatus()
  },
)

// 手动重新同步按钮点击
function handleResyncClick() {
  emit('resync')
}
</script>

<template>
  <n-drawer
    v-model:show="drawerVisible"
    placement="right"
    :width="420"
    :trap-focus="false"
  >
    <n-drawer-content>
      <template #header>
        <div class="flex items-center gap-2">
          <div :class="statusIcon" class="w-4 h-4" />
          <span class="text-sm font-medium">代码索引状态</span>
        </div>
      </template>

      <div class="space-y-4 text-xs">
        <!-- 项目基础信息 -->
        <div class="space-y-1">
          <div class="flex items-center justify-between">
            <span class="text-gray-500">项目路径</span>
            <span class="ml-2 truncate max-w-[260px]" :title="projectRoot">
              {{ projectRoot || '未提供' }}
            </span>
          </div>
          <div class="flex items-center justify-between">
            <span class="text-gray-500">整体状态</span>
            <span class="ml-2 font-medium">
              {{ statusSummary }}
            </span>
          </div>
          <div v-if="projectStatus" class="flex items-center justify-between">
            <span class="text-gray-500">进度</span>
            <span class="ml-2">
              {{ projectStatus.progress }}%
              <span class="ml-1 text-gray-500">
                ({{ projectStatus.indexed_files }}/{{ projectStatus.total_files }})
              </span>
            </span>
          </div>
        </div>

        <!-- 总体进度条 -->
        <div v-if="projectStatus">
          <n-progress
            type="line"
            :percentage="projectStatus.progress"
            :height="6"
            :border-radius="3"
            :show-indicator="false"
            :status="projectStatus.status === 'failed' ? 'error' : 'info'"
          />
        </div>

        <!-- 项目结构树 -->
        <div class="space-y-2">
          <div class="flex items-center justify-between">
            <span class="text-xs font-medium text-gray-500">项目结构</span>
            <n-button
              text
              size="tiny"
              :loading="loadingFiles"
              @click="fetchFilesStatus"
            >
              <template #icon>
                <div class="i-carbon-renew w-3 h-3" />
              </template>
              刷新
            </n-button>
          </div>

          <div class="min-h-[120px] max-h-[260px] overflow-y-auto pr-1">
            <div v-if="loadingFiles" class="flex items-center justify-center py-6">
              <n-spin size="small" />
            </div>

            <div v-else-if="filesError" class="text-red-500 py-2">
              {{ filesError }}
            </div>

            <div v-else-if="!treeData.length" class="text-gray-500 py-2">
              暂无可索引文件，请确认扩展名和排除规则配置是否正确。
            </div>

            <div v-else>
              <n-tree
                :data="treeData"
                :block-line="true"
                :selectable="false"
                :expand-on-click="true"
              />
            </div>
          </div>
        </div>

        <!-- 手动重新同步控制 -->
        <div class="pt-2 border-t border-gray-200 flex items-center justify-between gap-3">
          <div class="text-[11px] text-gray-500 leading-snug space-y-0.5">
            <div>重新同步会在后台执行，不会阻塞当前对话。</div>
            <div v-if="projectStatus?.last_success_time">
              上次成功：{{ projectStatus.last_success_time }}
            </div>
            <div v-if="projectStatus?.failed_files">
              失败文件数：<span class="text-red-500">{{ projectStatus.failed_files }}</span>
            </div>
            <div v-if="projectStatus?.last_error" class="text-red-500">
              最近错误：{{ projectStatus.last_error }}
            </div>
          </div>
          <n-button
            type="primary"
            size="small"
            :loading="resyncLoading || isIndexing"
            :disabled="resyncLoading || isIndexing || !projectRoot"
            strong
            @click="handleResyncClick"
          >
            <template #icon>
              <div class="i-carbon-renew w-4 h-4" />
            </template>
            {{ isIndexing ? '索引中...' : '重新同步' }}
          </n-button>
        </div>
      </div>
    </n-drawer-content>
  </n-drawer>
</template>
