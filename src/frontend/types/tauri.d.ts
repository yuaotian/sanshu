declare module '@tauri-apps/plugin-shell' {
  export function open(url: string): Promise<void>
}

// Acemcp 索引状态类型定义
export type IndexStatus = 'idle' | 'indexing' | 'synced' | 'failed'

export interface ProjectIndexStatus {
  project_root: string
  status: IndexStatus
  progress: number
  total_files: number
  indexed_files: number
  pending_files: number
  failed_files: number
  last_success_time: string | null
  last_failure_time: string | null
  last_error: string | null
  directory_stats: Record<string, [number, number]> // 目录路径 -> [总文件数, 已索引文件数]
}

export interface ProjectsIndexStatus {
  projects: Record<string, ProjectIndexStatus>
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
