<script setup lang="ts">
import { invoke } from '@tauri-apps/api/core'
import { useDialog, useMessage } from 'naive-ui'
import { computed, onMounted, onUnmounted, ref } from 'vue'
import { useAcemcpSync } from '../../composables/useAcemcpSync'
import type { IndexStatus, ProjectIndexStatus } from '../../types/tauri'
import { ProjectCard, ProjectCardSkeleton } from '../index'
import McpIndexStatusDrawer from '../popup/McpIndexStatusDrawer.vue'

// 使用 Acemcp 同步状态管理
const { triggerIndexUpdate } = useAcemcpSync()

const message = useMessage()
const dialog = useDialog()

/**
 * 规范化 Windows 路径
 * 去除扩展长度路径前缀 (\\?\ 或 //?/) 并统一使用正斜杠
 */
function normalizePath(path: string): string {
  let p = path || ''
  // 去除 Windows 扩展长度路径前缀
  if (p.startsWith('\\\\?\\')) {
    p = p.slice(4)
  }
  else if (p.startsWith('//?/')) {
    p = p.slice(4)
  }
  // 统一使用正斜杠
  return p.replace(/\\/g, '/')
}

// 本地状态
const loading = ref(true)
const allProjects = ref<Record<string, ProjectIndexStatus>>({})
const watchingProjects = ref<string[]>([])
const selectedProject = ref<string>('')
const showDrawer = ref(false)
const resyncLoading = ref(false)
// 目录存在状态缓存（key 为规范化后的路径）
const directoryExistsCache = ref<Record<string, boolean>>({})

// 搜索和筛选状态
const searchQuery = ref('')
const statusFilter = ref<IndexStatus | 'all' | 'stale'>('all')
const sortBy = ref<'status' | 'time' | 'name'>('status')

// 状态筛选选项
const statusOptions = [
  { label: '全部状态', value: 'all' },
  { label: '索引中', value: 'indexing' },
  { label: '待重建', value: 'stale' },
  { label: '已完成', value: 'synced' },
  { label: '失败', value: 'failed' },
  { label: '未索引', value: 'idle' },
]

// 排序选项
const sortOptions = [
  { label: '按状态', value: 'status' },
  { label: '按时间', value: 'time' },
  { label: '按名称', value: 'name' },
]

// 轮询定时器
let pollingTimer: number | null = null

// 选中项目的状态信息（用于抽屉组件）
const selectedProjectStatus = computed<ProjectIndexStatus | null>(() => {
  if (!selectedProject.value)
    return null
  return allProjects.value[selectedProject.value] || null
})

// 选中项目的状态摘要文本
const selectedStatusSummary = computed(() => {
  const status = selectedProjectStatus.value
  if (!status)
    return '未索引'
  if (status.is_stale && status.status !== 'indexing')
    return '配置已变更'
  switch (status.status) {
    case 'idle':
      return '空闲'
    case 'indexing':
      return `索引中 ${status.progress}%`
    case 'synced':
      return '已同步'
    case 'failed':
      return '索引失败'
    default:
      return '未知状态'
  }
})

// 选中项目的状态图标
const selectedStatusIcon = computed(() => {
  const selected = selectedProjectStatus.value
  if (selected?.is_stale && selected.status !== 'indexing')
    return 'i-carbon-warning-alt text-amber-500'

  const status = selected?.status
  switch (status) {
    case 'idle':
      return 'i-carbon-circle-dash text-gray-400'
    case 'indexing':
      return 'i-carbon-in-progress text-blue-500 animate-spin'
    case 'synced':
      return 'i-carbon-checkmark-filled text-green-500'
    case 'failed':
      return 'i-carbon-warning-filled text-red-500'
    default:
      return 'i-carbon-help text-gray-400'
  }
})

// 选中项目是否正在索引
const selectedIsIndexing = computed(() => {
  return selectedProjectStatus.value?.status === 'indexing'
})

// 是否有正在索引的项目（用于控制轮询频率）
const hasIndexingProject = computed(() => {
  return Object.values(allProjects.value).some(p => p.status === 'indexing')
})

// 计算项目列表（带搜索、筛选和排序）
const projectList = computed(() => {
  let list = Object.values(allProjects.value)

  // 搜索过滤
  if (searchQuery.value.trim()) {
    const query = searchQuery.value.toLowerCase()
    list = list.filter(p => p.project_root.toLowerCase().includes(query))
  }

  // 状态筛选
  if (statusFilter.value !== 'all') {
    list = list.filter((p) => {
      if (statusFilter.value === 'stale')
        return !!p.is_stale && p.status !== 'indexing'
      return p.status === statusFilter.value
    })
  }

  // 排序
  const statusOrder = { indexing: 0, stale: 1, synced: 2, failed: 3, idle: 4 }
  list.sort((a, b) => {
    switch (sortBy.value) {
      case 'status':
        return statusOrder[a.is_stale && a.status !== 'indexing' ? 'stale' : a.status]
          - statusOrder[b.is_stale && b.status !== 'indexing' ? 'stale' : b.status]
      case 'time': {
        const timeA = a.last_success_time ? new Date(a.last_success_time).getTime() : 0
        const timeB = b.last_success_time ? new Date(b.last_success_time).getTime() : 0
        return timeB - timeA // 最近的在前
      }
      case 'name': {
        const nameA = a.project_root.split(/[/\\]/).pop() || ''
        const nameB = b.project_root.split(/[/\\]/).pop() || ''
        return nameA.localeCompare(nameB)
      }
      default:
        return 0
    }
  })

  return list
})

// 统计信息
const stats = computed(() => {
  const projects = Object.values(allProjects.value)
  return {
    total: projects.length,
    indexing: projects.filter(p => p.status === 'indexing').length,
    stale: projects.filter(p => p.is_stale && p.status !== 'indexing').length,
    synced: projects.filter(p => p.status === 'synced' && !p.is_stale).length,
    failed: projects.filter(p => p.status === 'failed').length,
  }
})

// 初始化加载
onMounted(async () => {
  await loadAllData()
  // 加载完成后检测所有目录的存在状态
  await checkAllDirectoriesExist()
  startPolling()
})

// 组件卸载时清理
onUnmounted(() => {
  stopPolling()
})

// 开始轮询
function startPolling() {
  if (pollingTimer)
    return
  // 根据是否有索引中的项目调整轮询频率
  const interval = hasIndexingProject.value ? 3000 : 30000
  pollingTimer = window.setInterval(async () => {
    await refreshData()
    // 动态调整轮询频率
    if (pollingTimer) {
      const newInterval = hasIndexingProject.value ? 3000 : 30000
      if (newInterval !== interval) {
        stopPolling()
        startPolling()
      }
    }
  }, interval)
}

// 停止轮询
function stopPolling() {
  if (pollingTimer) {
    clearInterval(pollingTimer)
    pollingTimer = null
  }
}

// 刷新数据（不显示加载状态）
async function refreshData() {
  try {
    const [statusResult, watchingResult] = await Promise.all([
      invoke<{ projects: Record<string, ProjectIndexStatus> }>('get_all_acemcp_index_status'),
      invoke<string[]>('get_watching_projects'),
    ])
    allProjects.value = statusResult.projects
    watchingProjects.value = watchingResult
  }
  catch (err) {
    console.error('刷新项目索引数据失败:', err)
  }
}

// 加载所有数据（显示加载状态）
async function loadAllData() {
  loading.value = true
  try {
    const [statusResult, watchingResult] = await Promise.all([
      invoke<{ projects: Record<string, ProjectIndexStatus> }>('get_all_acemcp_index_status'),
      invoke<string[]>('get_watching_projects'),
    ])
    allProjects.value = statusResult.projects
    watchingProjects.value = watchingResult
  }
  catch (err) {
    console.error('加载项目索引数据失败:', err)
    message.error('加载项目索引数据失败')
  }
  finally {
    loading.value = false
  }
}

// 复制项目路径（直接使用子组件传递的规范化路径）
async function copyPath(path: string) {
  try {
    await navigator.clipboard.writeText(path)
    message.success('路径已复制到剪贴板')
  }
  catch (err) {
    message.error('复制失败')
  }
}

// 切换项目监听状态
async function toggleWatching(projectRoot: string) {
  // 规范化路径，去除 Windows 扩展前缀
  const normalizedPath = normalizePath(projectRoot)
  const currentlyWatching = watchingProjects.value.some(p => normalizePath(p) === normalizedPath)
  try {
    if (currentlyWatching) {
      await invoke('stop_project_watching', { projectRootPath: normalizedPath })
      message.success('已停止监听项目')
    }
    else {
      // 修正：调用启动监听命令而非手动索引
      await invoke('start_project_watching', { projectRootPath: normalizedPath })
      message.success('已开启监听项目')
    }
    watchingProjects.value = await invoke<string[]>('get_watching_projects')
  }
  catch (err) {
    console.error('切换监听状态失败:', err)
    message.error(`操作失败: ${err}`)
  }
}

// 重新索引（带二次确认）
function handleReindex(projectRoot: string) {
  const normalizedPath = normalizePath(projectRoot)
  dialog.warning({
    title: '确认重新索引',
    content: `确定要重新索引项目吗？\n\n${normalizedPath}\n\n这将重新扫描所有文件并更新索引。`,
    positiveText: '确认',
    negativeText: '取消',
    onPositiveClick: async () => {
      try {
        await triggerIndexUpdate(normalizedPath)
        message.success('已触发重新索引')
        // 延迟刷新状态
        setTimeout(() => loadAllData(), 1000)
      }
      catch (err) {
        console.error('重新索引失败:', err)
        message.error('重新索引失败')
      }
    },
  })
}

// 查看项目结构树
function viewProjectTree(projectRoot: string) {
  selectedProject.value = normalizePath(projectRoot)
  showDrawer.value = true
}

// 抽屉中的重新同步处理
async function handleDrawerResync() {
  if (!selectedProject.value)
    return
  resyncLoading.value = true
  try {
    // selectedProject 已经是规范化的路径
    await triggerIndexUpdate(selectedProject.value)
    message.success('已触发重新索引')
    // 延迟刷新状态
    setTimeout(() => loadAllData(), 1000)
  }
  catch (err) {
    console.error('重新索引失败:', err)
    message.error(`重新索引失败: ${err}`)
  }
  finally {
    resyncLoading.value = false
  }
}

// 检测目录是否存在
async function checkDirectoryExists(projectRoot: string): Promise<boolean> {
  const normalizedPath = normalizePath(projectRoot)
  // 优先使用缓存
  if (normalizedPath in directoryExistsCache.value) {
    return directoryExistsCache.value[normalizedPath]
  }
  try {
    const exists = await invoke<boolean>('check_directory_exists', {
      directoryPath: normalizedPath,
    })
    directoryExistsCache.value[normalizedPath] = exists
    return exists
  }
  catch (err) {
    console.error('检测目录存在性失败:', err)
    return true // 默认存在，避免误删
  }
}

// 加载所有项目的目录存在状态
async function checkAllDirectoriesExist() {
  const projects = Object.keys(allProjects.value)
  const results = await Promise.all(
    projects.map(async (projectRoot) => {
      const exists = await checkDirectoryExists(projectRoot)
      return { projectRoot, exists }
    }),
  )
  results.forEach(({ projectRoot, exists }) => {
    const normalizedPath = normalizePath(projectRoot)
    directoryExistsCache.value[normalizedPath] = exists
  })
}

// 删除项目索引记录（带二次确认）
function handleDeleteProject(projectRoot: string) {
  const normalizedPath = normalizePath(projectRoot)
  const projectName = normalizedPath.split('/').pop() || normalizedPath

  dialog.warning({
    title: '确认删除',
    content: `确定要删除项目索引记录吗？\n\n项目: ${projectName}\n路径: ${normalizedPath}\n\n此操作将从列表中移除该项目，不会删除实际文件。`,
    positiveText: '删除',
    negativeText: '取消',
    onPositiveClick: async () => {
      try {
        console.debug('[handleDeleteProject] 开始删除, normalizedPath=', normalizedPath)
        const result = await invoke<string>('remove_acemcp_project_index', {
          projectRootPath: normalizedPath,
        })
        console.debug('[handleDeleteProject] 删除命令执行结果:', result)
        message.success('已删除项目索引记录')
        // 从本地缓存中移除
        delete directoryExistsCache.value[normalizedPath]
        // 刷新列表
        console.debug('[handleDeleteProject] 开始刷新列表')
        await loadAllData()
        console.debug('[handleDeleteProject] 刷新完成, 项目数=', Object.keys(allProjects.value).length)
      }
      catch (err) {
        console.error('删除项目索引记录失败:', err)
        message.error(`删除失败: ${err}`)
      }
    },
  })
}

// 获取指定项目的目录存在状态
function getDirectoryExists(projectRoot: string): boolean {
  const normalizedPath = normalizePath(projectRoot)
  // 如果还没检测过，默认返回 true
  return directoryExistsCache.value[normalizedPath] ?? true
}
</script>

<template>
  <div>
    <!-- 顶部工具栏 -->
    <div class="mb-4 space-y-3">
      <!-- 统计信息 -->
      <div class="flex flex-wrap gap-3">
        <n-tag size="small" :bordered="false">
          <template #icon><div class="i-carbon-folder" /></template>
          {{ stats.total }} 个项目
        </n-tag>
        <n-tag v-if="stats.indexing > 0" size="small" type="info" :bordered="false">
          <template #icon><div class="i-carbon-in-progress animate-spin" /></template>
          {{ stats.indexing }} 索引中
        </n-tag>
        <n-tag v-if="stats.stale > 0" size="small" type="warning" :bordered="false">
          <template #icon><div class="i-carbon-warning-alt" /></template>
          {{ stats.stale }} 待重建
        </n-tag>
        <n-tag v-if="stats.synced > 0" size="small" type="success" :bordered="false">
          <template #icon><div class="i-carbon-checkmark-filled" /></template>
          {{ stats.synced }} 已完成
        </n-tag>
        <n-tag v-if="stats.failed > 0" size="small" type="error" :bordered="false">
          <template #icon><div class="i-carbon-warning-filled" /></template>
          {{ stats.failed }} 失败
        </n-tag>
      </div>

      <!-- 搜索和筛选 -->
      <div class="flex flex-wrap gap-2 items-center">
        <n-input
          v-model:value="searchQuery"
          placeholder="搜索项目..."
          clearable
          size="small"
          class="flex-1 min-w-[150px] max-w-[250px]"
        >
          <template #prefix>
            <div class="i-carbon-search opacity-50" />
          </template>
        </n-input>

        <n-select
          v-model:value="statusFilter"
          :options="statusOptions"
          size="small"
          class="w-[100px]"
          placeholder="状态"
        />

        <n-select
          v-model:value="sortBy"
          :options="sortOptions"
          size="small"
          class="w-[90px]"
          placeholder="排序"
        />

        <n-button size="small" quaternary @click="loadAllData">
          <template #icon>
            <div class="i-carbon-renew" />
          </template>
        </n-button>
      </div>
    </div>

    <!-- 加载状态 - 骨架屏网格 -->
    <div v-if="loading" class="grid grid-cols-1 md:grid-cols-2 gap-4">
      <ProjectCardSkeleton v-for="i in 6" :key="i" />
    </div>

    <!-- 空状态 -->
    <n-empty
      v-else-if="projectList.length === 0 && !searchQuery && statusFilter === 'all'"
      description="使用代码搜索工具后，项目将自动显示在这里"
      class="py-12"
    >
      <template #icon>
        <div class="i-carbon-folder-off text-5xl opacity-30" />
      </template>
      <template #extra>
        <span class="text-base font-medium opacity-80">暂无项目索引数据</span>
      </template>
    </n-empty>

    <!-- 搜索无结果 -->
    <n-empty
      v-else-if="projectList.length === 0"
      description="尝试调整搜索条件或筛选器"
      class="py-12"
    >
      <template #icon>
        <div class="i-carbon-search text-4xl opacity-30" />
      </template>
      <template #extra>
        <n-button size="small" @click="searchQuery = ''; statusFilter = 'all'">
          清除筛选
        </n-button>
      </template>
    </n-empty>

    <!-- 项目卡片网格 -->
    <div v-else class="grid grid-cols-1 md:grid-cols-2 gap-4">
      <ProjectCard
        v-for="project in projectList"
        :key="project.project_root"
        :project="project"
        :is-watching="watchingProjects.some(p => normalizePath(p) === normalizePath(project.project_root))"
        :directory-exists="getDirectoryExists(project.project_root)"
        @view-tree="viewProjectTree(project.project_root)"
        @reindex="handleReindex(project.project_root)"
        @toggle-watching="toggleWatching(project.project_root)"
        @copy-path="copyPath"
        @delete="handleDeleteProject(project.project_root)"
      />
    </div>

    <!-- 项目结构树抽屉 -->
    <McpIndexStatusDrawer
      v-model:show="showDrawer"
      :project-root="selectedProject"
      :status-summary="selectedStatusSummary"
      :status-icon="selectedStatusIcon"
      :project-status="selectedProjectStatus"
      :is-indexing="selectedIsIndexing"
      :resync-loading="resyncLoading"
      @resync="handleDrawerResync"
    />
  </div>
</template>

