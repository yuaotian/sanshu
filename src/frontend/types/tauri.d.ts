declare module '@tauri-apps/plugin-shell' {
  export function open(url: string): Promise<void>
}

// Acemcp 索引状态类型定义
export type IndexStatus = 'idle' | 'indexing' | 'paused' | 'synced' | 'failed'

export type ProjectScopeRiskLevel = 'critical_path' | 'excessive_scale'

export interface ProjectScopeRisk {
  level: ProjectScopeRiskLevel
  reason_code: string
  reason: string
  scanned_entries: number
  candidate_files: number
  candidate_bytes: number
  project_markers: string[]
  requires_secondary_confirmation: boolean
  detected_at: string
}

export interface ProjectIndexStatus {
  is_workspace?: boolean
  workspace_project_count?: number
  workspace_children?: string[]
  workspace_indexing_project_count?: number
  workspace_paused_project_count?: number
  workspace_failed_project_count?: number
  workspace_resolution_error?: string | null
  project_root: string
  status: IndexStatus
  /** 已有部分 blob 可搜索，但仍有未确认内容。 */
  is_partial?: boolean
  progress: number
  total_files: number
  indexed_files: number
  pending_files: number
  failed_files: number
  last_success_time: string | null
  last_failure_time: string | null
  last_error: string | null
  last_failure_scope_hash?: string | null
  index_scope_hash?: string | null
  is_stale?: boolean
  stale_reason?: string | null
  directory_stats: Record<string, [number, number]> // 目录路径 -> [总文件数, 已索引文件数]
  recent_indexed_files?: string[] // 最近增量索引的文件列表（最多 5 个）
  job_id?: string | null
  total_batches?: number
  completed_batches?: number
  job_updated_at?: string | null
  scope_risk?: ProjectScopeRisk | null
}

export interface ProjectsIndexStatus {
  projects: Record<string, ProjectIndexStatus>
}

export interface LocalIndexScopeStatus {
  name: string
  relative_path: string
  project_root: string
  index_path: string
  state: 'missing' | 'building' | 'ready' | 'partial' | 'error'
  indexed_files: number
  indexed_chunks: number
  lexical_sync_running: boolean
  pending_changes: boolean
  semantic_state: 'disabled' | 'missing' | 'building' | 'syncing' | 'ready' | 'partial' | 'error'
  semantic_indexed_chunks: number
  semantic_pending_chunks: number
  last_error?: string
}

export interface LocalIndexStatus {
  is_workspace: boolean
  project_count: number
  scopes: LocalIndexScopeStatus[]
  project_root: string
  index_path: string
  state: 'missing' | 'building' | 'ready' | 'partial' | 'error'
  indexed_files: number
  indexed_chunks: number
  sync_running: boolean
  lexical_sync_running: boolean
  semantic_sync_running: boolean
  pending_changes: boolean
  last_error?: string
  semantic_state: 'disabled' | 'missing' | 'building' | 'syncing' | 'ready' | 'partial' | 'error'
  semantic_model?: string
  semantic_indexed_chunks: number
  semantic_pending_chunks: number
  semantic_last_error?: string
  semantic_requested_provider?: 'auto' | 'cuda' | 'cpu' | string
  semantic_execution_provider?: 'cuda' | 'cpu' | string
  semantic_provider_fallback_reason?: string
  semantic_cuda_runtime_available?: boolean
  semantic_cuda_runtime_dir?: string
  semantic_cuda_runtime_error?: string
  semantic_batch_size?: number
  semantic_intra_threads?: number
}

// Acemcp 文件级索引状态类型定义
export type FileIndexStatusType = 'indexed' | 'pending'

export interface FileIndexStatus {
  path: string
  status: FileIndexStatusType
}

export interface ProjectFilesStatus {
  project_root: string
  files: FileIndexStatus[]
}

// 嵌套项目信息（检测到的子目录中的独立 Git 仓库）
export interface NestedProjectInfo {
  // 子项目路径（相对于父项目根目录）
  relative_path: string
  // 子项目绝对路径
  absolute_path: string
  // 是否是独立的 Git 仓库
  is_git_repo: boolean
  // 子项目的索引状态
  index_status: ProjectIndexStatus | null
  // 子项目包含的文件数量
  file_count: number
}

// 包含嵌套项目信息的项目状态
export interface ProjectWithNestedStatus {
  // 主项目的索引状态
  root_status: ProjectIndexStatus
  // 检测到的嵌套项目列表
  nested_projects: NestedProjectInfo[]
  // 普通子目录列表（不含 .git）
  regular_directories: string[]
}
