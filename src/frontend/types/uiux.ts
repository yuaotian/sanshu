// 中文说明：只在当前工具配置页生命周期内交接草稿，不持久化模型状态或异步任务。
export interface UiuxConfigData {
  knowledge_backend: 'auto' | 'fast_context' | 'local'
  semantic_enabled: boolean
  model_dir: string | null
  effective_model_dir: string
}

export interface UiuxEditSession {
  saved: UiuxConfigData
  draft: UiuxConfigData
}
