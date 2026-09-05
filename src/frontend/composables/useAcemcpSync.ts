import type { ProjectIndexStatus, ProjectsIndexStatus, ProjectWithNestedStatus } from '../types/tauri'
import { invoke } from '@tauri-apps/api/core'
import { computed, onUnmounted, ref } from 'vue'

/**
 * 规范化项目路径，去除 Windows 扩展路径前缀并统一使用正斜杠
 * 确保前后端路径格式一致，避免 HashMap 查找失败
 */
function normalizeProjectPath(path: string): string {
  let p = path
  // 处理 //?/ 格式
  if (p.startsWith('//?/'))
    p = p.slice(4)
  // 处理 \\?\ 格式
  else if (p.startsWith('\\\\?\\'))
    p = p.slice(4)
  // 统一使用正斜杠
  return p.replace(/\\/g, '/')
}

// 全局状态
const allProjectsStatus = ref<ProjectsIndexStatus>({ projects: {} })
const currentProjectRoot = ref<string>('')
const isPolling = ref(false)
const autoIndexEnabled = ref(true)
const watchingProjects = ref<string[]>([])

let pollingTimer: number | null = null

/**
 * Acemcp 索引同步状态管理
 */
export function useAcemcpSync() {
  // 当前项目的索引状态
  // 使用路径规范化查找，支持原始路径和规范化路径匹配
  const currentProjectStatus = computed<ProjectIndexStatus | null>(() => {
    if (!currentProjectRoot.value)
      return null

    const projects = allProjectsStatus.value.projects
    const rawPath = currentProjectRoot.value
    const normalizedPath = normalizeProjectPath(rawPath)

    // 尝试直接匹配原始路径
    if (projects[rawPath])
      return projects[rawPath]

    // 尝试匹配规范化后的路径
    if (projects[normalizedPath])
      return projects[normalizedPath]

    // 遍历所有项目，尝试规范化后匹配
    for (const [key, status] of Object.entries(projects)) {
      if (normalizeProjectPath(key) === normalizedPath)
        return status
    }

    return null
  })

  function isAuthFailureStatus(status: ProjectIndexStatus | null): boolean {
    if (!status || status.status !== 'failed')
      return false

    const lastError = status.last_error || ''
    const lower = lastError.toLowerCase()
    return lower.includes('401') || lower.includes('认证失败') || lower.includes('invalid token')
  }

  // 状态摘要文本
  const statusSummary = computed(() => {
    const status = currentProjectStatus.value
    if (!status)
      return '未索引'
    if (status.is_partial)
      return `部分可用 ${status.indexed_files}/${status.total_files}`

    switch (status.status) {
      case 'idle':
        return '空闲'
      case 'indexing':
        return `索引中 ${status.progress}%`
      case 'paused':
        return `等待恢复 ${status.progress}%`
      case 'synced':
        return '已同步'
      case 'failed':
        return isAuthFailureStatus(status) ? 'Token 已失效' : '索引失败'
      default:
        return '未知状态'
    }
  })

  // 状态图标类名
  const statusIcon = computed(() => {
    const status = currentProjectStatus.value?.status
    if (currentProjectStatus.value?.is_partial)
      return 'i-carbon-warning-alt text-amber-500'
    switch (status) {
      case 'idle':
        return 'i-carbon-circle-dash text-gray-400'
      case 'indexing':
        return 'i-carbon-in-progress text-blue-500 animate-spin'
      case 'paused':
        return 'i-carbon-pause-outline text-amber-500'
      case 'synced':
        return 'i-carbon-checkmark-filled text-green-500'
      case 'failed':
        return isAuthFailureStatus(currentProjectStatus.value)
          ? 'i-carbon-warning-alt-filled text-red-500'
          : 'i-carbon-warning-filled text-red-500'
      default:
        return 'i-carbon-help text-gray-400'
    }
  })

  // 是否正在索引
  const isIndexing = computed(() => {
    return currentProjectStatus.value?.status === 'indexing'
  })

  // 获取所有项目的索引状态
  async function fetchAllStatus() {
    try {
      const result = await invoke<ProjectsIndexStatus>('get_all_acemcp_index_status')
      allProjectsStatus.value = result
    }
    catch (err) {
      console.error('获取索引状态失败:', err)
    }
  }

  // 获取指定项目的索引状态
  async function fetchProjectStatus(projectRoot: string) {
    try {
      const result = await invoke<ProjectIndexStatus>('get_acemcp_index_status', {
        projectRootPath: projectRoot,
      })
      // 更新到全局状态中
      allProjectsStatus.value.projects[projectRoot] = result
    }
    catch (err) {
      console.error('获取项目索引状态失败:', err)
    }
  }

  // 手动触发索引更新（增量/全量）
  async function triggerIndexUpdate(projectRoot: string, mode: 'incremental' | 'full' = 'incremental') {
    try {
      const command = mode === 'full'
        ? 'trigger_acemcp_index_rebuild'
        : 'trigger_acemcp_index_update'
      const result = await invoke<string>(command, {
        projectRootPath: projectRoot,
      })
      // 立即刷新状态
      await fetchProjectStatus(projectRoot)
      return result
    }
    catch (err) {
      throw new Error(`触发索引更新失败: ${err}`)
    }
  }

  // 获取全局自动索引开关状态
  async function fetchAutoIndexEnabled() {
    try {
      const result = await invoke<boolean>('get_auto_index_enabled')
      autoIndexEnabled.value = result
    }
    catch (err) {
      console.error('获取自动索引开关状态失败:', err)
    }
  }

  // 设置全局自动索引开关
  async function setAutoIndexEnabled(enabled: boolean) {
    try {
      await invoke('set_auto_index_enabled', { enabled })
      autoIndexEnabled.value = enabled
    }
    catch (err) {
      throw new Error(`设置自动索引开关失败: ${err}`)
    }
  }

  // 获取正在监听的项目列表
  async function fetchWatchingProjects() {
    try {
      const result = await invoke<string[]>('get_watching_projects')
      watchingProjects.value = result
    }
    catch (err) {
      console.error('获取监听项目列表失败:', err)
    }
  }

  // 检查项目是否正在监听
  async function isProjectWatching(projectRoot: string) {
    try {
      return await invoke<boolean>('is_project_watching', {
        projectRootPath: projectRoot,
      })
    }
    catch (err) {
      console.error('检查项目监听状态失败:', err)
      return false
    }
  }

  // 停止监听指定项目
  async function stopProjectWatching(projectRoot: string) {
    try {
      await invoke('stop_project_watching', {
        projectRootPath: projectRoot,
      })
      await fetchWatchingProjects()
    }
    catch (err) {
      throw new Error(`停止项目监听失败: ${err}`)
    }
  }

  // 停止所有监听
  async function stopAllWatching() {
    try {
      await invoke('stop_all_watching')
      watchingProjects.value = []
    }
    catch (err) {
      throw new Error(`停止所有监听失败: ${err}`)
    }
  }

  // 开始轮询（用于 MCP 弹窗）
  function startPolling(projectRoot?: string, intervalMs = 3000) {
    if (isPolling.value)
      return

    if (projectRoot)
      currentProjectRoot.value = projectRoot

    isPolling.value = true

    // 立即执行一次
    fetchAllStatus()
    fetchAutoIndexEnabled()
    fetchWatchingProjects()

    // 定时轮询
    pollingTimer = window.setInterval(() => {
      fetchAllStatus()
      fetchWatchingProjects()
    }, intervalMs)
  }

  // 停止轮询
  function stopPolling() {
    if (pollingTimer !== null) {
      clearInterval(pollingTimer)
      pollingTimer = null
    }
    isPolling.value = false
  }

  // 设置当前项目
  function setCurrentProject(projectRoot: string) {
    currentProjectRoot.value = projectRoot
  }

  // 组件卸载时清理
  onUnmounted(() => {
    stopPolling()
  })

  // 检测 ACE 配置是否完整（base_url 和 token 均已配置）
  async function checkAcemcpConfigured(): Promise<boolean> {
    try {
      const config = await invoke<{ base_url?: string, token?: string }>('get_acemcp_config')
      return !!(config.base_url && config.token)
    }
    catch (err) {
      console.error('检测 ACE 配置失败:', err)
      return false
    }
  }

  // 获取项目及其嵌套子项目的索引状态
  async function fetchProjectWithNested(projectRoot: string) {
    try {
      const result = await invoke<ProjectWithNestedStatus>('get_acemcp_project_with_nested', {
        projectRootPath: projectRoot,
      })
      return result
    }
    catch (err) {
      console.error('获取嵌套项目状态失败:', err)
      return null
    }
  }

  return {
    // 状态
    allProjectsStatus,
    currentProjectStatus,
    currentProjectRoot,
    isPolling,
    autoIndexEnabled,
    watchingProjects,

    // 计算属性
    statusSummary,
    statusIcon,
    isIndexing,

    // 方法
    fetchAllStatus,
    fetchProjectStatus,
    triggerIndexUpdate,
    fetchAutoIndexEnabled,
    setAutoIndexEnabled,
    fetchWatchingProjects,
    isProjectWatching,
    stopProjectWatching,
    stopAllWatching,
    startPolling,
    stopPolling,
    setCurrentProject,
    checkAcemcpConfigured,
    fetchProjectWithNested,
    normalizeProjectPath,
  }
}
