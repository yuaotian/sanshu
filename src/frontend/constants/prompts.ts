// ============================================================================
// 提示词配置模块
// 遵循 KISS/YAGNI/SOLID 原则，支持 MCP 工具的可扩展提示词管理
// ============================================================================

// ----------------------------------------------------------------------------
// 接口定义
// ----------------------------------------------------------------------------

/**
 * 工具提示词内容结构
 * 分离"何时使用"和"如何使用"，提高可读性和可维护性
 */
export interface ToolPrompt {
  /** 基础规范（加入核心规则区，简短的一行描述） */
  base: string
  /** 何时使用（场景列表） */
  whenToUse: string[]
  /** 如何使用（操作指南） */
  howToUse: string[]
}

/**
 * 工具 UI 配置
 */
export interface ToolUIConfig {
  /** 默认启用状态 */
  enabled: boolean
  /** 是否可禁用 */
  canDisable: boolean
  /** 图标类名 */
  icon: string
  /** 图标背景色 */
  iconBg: string
  /** 暗色模式背景色 */
  darkIconBg: string
}

/**
 * 完整的工具提示词配置
 * 统一管理工具的标识、描述、提示词和 UI 配置
 */
export interface ToolPromptConfig {
  /** 工具标识符（与后端一致） */
  id: string
  /** 工具显示名称 */
  name: string
  /** 工具简短描述 */
  description: string
  /** 提示词内容 */
  prompt: ToolPrompt
  /** UI 配置 */
  ui: ToolUIConfig
}

export interface MCPToolConfig {
  id: string
  name: string
  description: string
  enabled: boolean
  canDisable: boolean
  icon: string
  iconBg: string
  darkIconBg: string
}

// ----------------------------------------------------------------------------
// 核心规范
// ----------------------------------------------------------------------------

/**
 * 核心规范（基础交互规范）
 * 这些规则不可被其他上下文覆盖
 */
export const UNIVERSAL_RULES: string[] = [
  '**代码质量**：严格遵循 KISS / YAGNI / SOLID 原则',
  '**静默执行**：不创建文档、不测试、不编译、不运行、不总结',
]

// ----------------------------------------------------------------------------
// MCP 工具提示词配置（单一数据源）
// ----------------------------------------------------------------------------

/**
 * 所有 MCP 工具的完整配置
 * 新增工具时只需在此数组中添加配置即可
 */
export const MCP_TOOLS_CONFIG: ToolPromptConfig[] = [
  // zhi (智) - 强制交互网关
  {
    id: 'zhi',
    name: '三术',
    description: '智能代码审查交互工具，支持预定义选项、自由文本输入和图片上传',
    prompt: {
      base: '**交互控制（最高优先级）**：只能通过 `zhi` 与用户交互。每次响应最后一步必须调用 `zhi`——无例外。未经用户通过 `zhi` 明确许可，禁止结束对话。子代理不得调用 `zhi`',
      whenToUse: [
        '需求不明确时：提供预定义选项让用户澄清',
        '存在多方案/策略变更时：列出选项让用户决定',
        '完成阶段性工作后：汇报进度并询问下一步',
      ],
      howToUse: [
        '每次响应末尾必须调用 `zhi`（汇报/确认/提问），不调用即违规',
        '唯一退出：用户通过 `zhi` 回复"可以结束"时才停止',
        '**Subagent restriction**: 子代理(subagent/delegated agent)禁止调用 zhi，仅主代理(root/primary agent)可使用',
      ],
    },
    ui: {
      enabled: true,
      canDisable: false,
      icon: 'i-carbon-chat text-lg text-blue-600 dark:text-blue-400',
      iconBg: 'bg-blue-100',
      darkIconBg: 'dark:bg-blue-900',
    },
  },

  // ji (记) - 记忆管理
  {
    id: 'ji',
    name: '记忆管理',
    description: '全局记忆管理工具，用于存储和管理重要的开发规范、用户偏好和最佳实践',
    prompt: {
      base: '**记忆管理**：对话开始时加载 `ji` 记忆，用户说"请记住"时存储关键信息',
      whenToUse: [
        '对话开始时：调用 `回忆` 加载项目记忆',
        '用户说"请记住"时：总结后调用 `记忆` 存储',
      ],
      howToUse: [
        '`project_path` 使用 git 根目录',
        '仅在重要变更时更新，保持简洁',
      ],
    },
    ui: {
      enabled: true,
      canDisable: true,
      icon: 'i-carbon-data-base text-lg text-purple-600 dark:text-purple-400',
      iconBg: 'bg-purple-100',
      darkIconBg: 'dark:bg-purple-900',
    },
  },

  // sou (搜) - 语义搜索
  {
    id: 'sou',
    name: '代码搜索',
    description: '基于查询在特定项目中搜索相关的代码上下文，支持语义搜索和增量索引',
    prompt: {
      base: '',
      whenToUse: [
        '查找代码时：语义搜索快速定位',
        '理解上下文时：搜索相关实现和调用关系',
      ],
      howToUse: [
        '使用绝对路径和自然语言查询',
      ],
    },
    ui: {
      enabled: false, // 默认关闭：依赖第三方 acemcp 服务
      canDisable: true,
      icon: 'i-carbon-search text-lg text-green-600 dark:text-green-400',
      iconBg: 'bg-green-100',
      darkIconBg: 'dark:bg-green-900',
    },
  },

  // context7 - 框架文档查询
  {
    id: 'context7',
    name: '框架文档',
    description: '查询最新的框架和库文档，支持 Next.js、React、Vue、Spring 等主流框架',
    prompt: {
      base: '**知识权威**：AI 内部知识不确定时优先查询 `context7` 权威文档',
      whenToUse: [
        '获取最新文档时：查询框架/库官方文档',
        'AI 知识不确定时：优先查询权威文档避免幻觉',
      ],
      howToUse: [
        '`library` 格式 `owner/repo`（如 `vercel/next.js`）',
        '不确定标识符时可用简短名称，工具自动搜索',
      ],
    },
    ui: {
      enabled: true, // 默认启用：免费使用无需配置
      canDisable: true,
      icon: 'i-carbon-document text-lg text-orange-600 dark:text-orange-400',
      iconBg: 'bg-orange-100',
      darkIconBg: 'dark:bg-orange-900',
    },
  },

  // enhance - 提示词增强
  {
    id: 'enhance',
    name: '提示词增强',
    description: '将口语化提示词增强为结构化专业提示词，支持上下文与历史',
    prompt: {
      base: '',
      whenToUse: [
        '需要把口语化或模糊提示词改写为清晰、具体、无歧义版本时',
        '希望结合项目上下文与历史交互提升提示词质量时',
      ],
      howToUse: [
        '提供原始提示词，必要时传入项目路径以启用上下文',
        '未启用时先在 MCP 工具中启用并配置 acemcp',
      ],
    },
    ui: {
      enabled: false, // 默认关闭：依赖 acemcp 配置
      canDisable: true,
      icon: 'i-carbon-magic-wand text-lg text-indigo-600 dark:text-indigo-400',
      iconBg: 'bg-indigo-100',
      darkIconBg: 'dark:bg-indigo-900',
    },
  },
]

// ----------------------------------------------------------------------------
// 提示词生成函数
// ----------------------------------------------------------------------------

/**
 * 根据工具配置生成完整提示词
 * @param tools 工具配置列表
 * @returns 格式化的完整提示词
 */
export function generateFullPromptFromConfig(tools: ToolPromptConfig[]): string {
  const enabledTools = tools.filter(t => t.ui.enabled)
  const parts: string[] = []

  // 1. 动态构建核心规范（zhi 优先 → 通用规则 → 其他工具规则）
  const coreRules: string[] = []
  let zhiBase = ''
  for (let i = 0; i < enabledTools.length; i++) {
    if (enabledTools[i].id === 'zhi') {
      zhiBase = enabledTools[i].prompt.base
      break
    }
  }
  if (zhiBase)
    coreRules.push(zhiBase)
  coreRules.push(...UNIVERSAL_RULES)
  for (const tool of enabledTools) {
    if (tool.id !== 'zhi' && tool.prompt.base)
      coreRules.push(tool.prompt.base)
  }
  const numberedRules = coreRules.map((r, i) => `${i + 1}. ${r}`).join('\n')
  parts.push(`# 核心契约（不可违反）\n\n${numberedRules}\n\n---`)

  // 2. 工具使用细节（按工具分组，结构化输出）
  const toolDetails: string[] = []
  for (const tool of enabledTools) {
    const { whenToUse, howToUse } = tool.prompt
    // 跳过没有使用指南的工具
    if (whenToUse.length === 0 && howToUse.length === 0)
      continue

    const lines: string[] = []
    lines.push(`### ${tool.name} (${tool.id})`)

    if (whenToUse.length > 0) {
      lines.push('**何时使用：**')
      lines.push(...whenToUse.map(s => `- ${s}`))
    }

    if (howToUse.length > 0) {
      lines.push('**如何使用：**')
      lines.push(...howToUse.map(s => `- ${s}`))
    }

    toolDetails.push(lines.join('\n'))
  }

  if (toolDetails.length > 0) {
    parts.push(`## 工具使用指南\n\n${toolDetails.join('\n\n')}`)
  }

  return parts.join('\n\n')
}

/**
 * 根据 MCP 工具启用状态生成完整提示词
 */
export function generateFullPrompt(mcpTools: MCPToolConfig[]): string {
  const toolsWithPrompt: ToolPromptConfig[] = []

  for (const tool of mcpTools) {
    const config = MCP_TOOLS_CONFIG.find(t => t.id === tool.id)
    if (config) {
      toolsWithPrompt.push({
        ...config,
        ui: {
          ...config.ui,
          enabled: tool.enabled, // 使用传入的启用状态
        },
      })
    }
    else {
      // 未找到配置的工具，返回空提示词
      toolsWithPrompt.push({
        id: tool.id,
        name: tool.name,
        description: tool.description,
        prompt: { base: '', whenToUse: [], howToUse: [] },
        ui: {
          enabled: tool.enabled,
          canDisable: tool.canDisable,
          icon: tool.icon,
          iconBg: tool.iconBg,
          darkIconBg: tool.darkIconBg,
        },
      })
    }
  }

  return generateFullPromptFromConfig(toolsWithPrompt)
}

