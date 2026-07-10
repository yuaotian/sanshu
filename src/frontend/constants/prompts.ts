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

/**
 * 旧版提示词结构（向后兼容）
 */
export interface PromptSection {
  base: string
  detail: string
}

/**
 * 旧版 MCP 工具配置接口（向后兼容）
 */
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
 *
 * 单一事实源对齐：与仓库根目录 sanshu_prompt_core.md 保持文案一致
 * 修改本常量时，请同步更新 sanshu_prompt_core.md（反之亦然）
 */
export const CORE_RULES = `
# 核心契约（不可违反）
1. **代码哲学**：严格遵循 KISS / YAGNI / SOLID；不做过度设计、不加用不到的抽象
2. **强制交互**：所有方案确认与任务收尾必须通过 \`三术\` / \`zhi\` 工具展示；未得到完成指令前禁止主动结束对话
3. **证据优先**：任何分析必须基于真实搜索/读取到的代码；先用 \`sou\` 语义定位，再用 Read/Grep 确认实现，严禁臆测
4. **知识权威**：需要框架/库的最新文档时优先用 \`context7\`，避免训练数据过时
5. **持久化记忆**：对话开始时调用 \`ji\` 加载项目记忆；只有用户原始输入明确说"请记住"，或 \`zhi\` 返回的 \`structured_content.context_blocks\` 标记 \`memory_policy=save\` 时，才允许总结后存储为合适分类；\`memory_policy=never\` 的本轮上下文禁止写入记忆
6. **输出规范**：注释/日志默认中文；修改点必须有中文注释解释意图（why）；引用代码给出 \`文件路径:行号\`

---
`

// ----------------------------------------------------------------------------
// MCP 工具提示词配置（单一数据源）
// ----------------------------------------------------------------------------

/**
 * 所有 MCP 工具的完整配置
 * 新增工具时只需在此数组中添加配置即可
 */
export const MCP_TOOLS_CONFIG: ToolPromptConfig[] = [
  // zhi (智) - 强制交互网关
  // 与 CORE_RULES 第 2 条「强制交互」对齐，此处只补充触发场景，不重复硬性约束
  {
    id: 'zhi',
    name: '三术',
    description: '强制交互网关：方案确认、候选项呈现、任务收尾',
    prompt: {
      base: '',
      whenToUse: [
        '多方案抉择：列出所有候选并标注推荐项',
        '计划变更 / 任务完成：请求用户确认',
      ],
      howToUse: [],
    },
    ui: {
      enabled: true,
      canDisable: false,
      icon: 'i-carbon-chat text-lg text-blue-600 dark:text-blue-400',
      iconBg: 'bg-blue-100',
      darkIconBg: 'dark:bg-blue-900',
    },
  },

  // plan - 工作区开发计划跟踪
  {
    id: 'plan',
    name: '开发计划跟踪',
    description: '按工作区维护当前开发执行计划与步骤状态',
    prompt: {
      base: '',
      whenToUse: [
        '用户启用计划跟踪上下文后，在开发开始前提交计划并持续更新状态',
      ],
      howToUse: [
        '`set` 完整替换计划；`update` 只更新单项状态；`get` 查询；`clear` 清空',
        '状态按 `pending -> in_progress -> completed` 单向推进，同一时间最多一个进行中步骤',
      ],
    },
    ui: {
      enabled: true,
      canDisable: true,
      icon: 'i-carbon-list-checked text-lg text-teal-600 dark:text-teal-400',
      iconBg: 'bg-teal-100',
      darkIconBg: 'dark:bg-teal-900',
    },
  },

  // ji (记) - 记忆管理
  {
    id: 'memory',
    name: '记忆管理',
    description: '项目级记忆库：规范、偏好、模式、上下文',
    prompt: {
      base: '',
      whenToUse: [
        '对话开始：加载 `project_path`（git 根目录）下的记忆',
        '用户原始输入说「请记住」：总结后存储为合适分类',
        '`zhi.structured_content.context_blocks` 中 `memory_policy=save`：按返回的 category 存储；`memory_policy=never` 不得存储',
      ],
      howToUse: [],
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
    description: '项目内语义搜索：定位实现、调用关系、上下文',
    prompt: {
      base: '',
      whenToUse: [
        '分析前必须先用 `sou` 定位证据，再用 Read/Grep 确认',
      ],
      howToUse: [
        '代码标识符通常为英文：用中文 query 时建议混入英文类名/函数名/文件名（如 `GestureRecognizer`、`ImageCodec`），可显著提升首轮命中率',
        '若第一次返回 0 文件，请拆成更具体的子问题或显式给出英文关键词重试',
        '给出模块/目录提示（如「gesture 模块」/「src/capture/」）有助于快速定位',
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
    description: '查询框架/库的最新官方文档（Next.js / React / Vue / Spring 等）',
    prompt: {
      base: '',
      whenToUse: [
        'AI 知识可能过时或不确定时优先查询，避免幻觉',
      ],
      howToUse: [
        '`library` 格式 `owner/repo`（如 `vercel/next.js`），不确定时可用短名',
      ],
    },
    ui: {
      enabled: true,
      canDisable: true,
      icon: 'i-carbon-document text-lg text-orange-600 dark:text-orange-400',
      iconBg: 'bg-orange-100',
      darkIconBg: 'dark:bg-orange-900',
    },
  },

  // uiux - UI/UX 美化与设计审查
  // 与 CORE_RULES 协同，此处仅补充触发场景与输出参考字段
  {
    id: 'uiux',
    name: 'UI/UX 工具',
    description: '页面美化、UI 描述、设计系统、UI 审查的单一入口',
    prompt: {
      base: '',
      whenToUse: [
        '涉及页面美化 / UI 描述 / 设计系统 / UI 审查时优先使用',
      ],
      howToUse: [
        '参考返回的 `prompt` / `uiux_hits` / `project_context` 三个字段',
      ],
    },
    ui: {
      enabled: true,
      canDisable: true,
      icon: 'i-carbon-color-palette text-lg text-pink-600 dark:text-pink-400',
      iconBg: 'bg-pink-100',
      darkIconBg: 'dark:bg-pink-900',
    },
  },

  // enhance - 提示词增强
  {
    id: 'enhance',
    name: '提示词增强',
    description: '把口语化提示词改写为结构化版本',
    prompt: {
      base: '',
      whenToUse: [
        '原始提示词模糊或口语化时',
      ],
      howToUse: [
        '依赖 acemcp 配置；传入项目路径可启用项目上下文',
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

  // tavily - AI 搜索与内容提取
  {
    id: 'tavily',
    name: 'Tavily AI 搜索',
    description: '实时 AI 搜索与内容提取：互联网最新信息、新闻、网页提取',
    prompt: {
      base: '',
      whenToUse: [
        '需要获取实时、最新的互联网信息时',
        '需要从指定 URL 提取结构化内容时',
      ],
      howToUse: [
        '搜索：传入 query 即可获取 AI 回答和多条搜索结果',
        '提取：action="extract" + urls 参数，从网页提取 Markdown 内容',
        '免费每月 1000 信用点；basic 搜索 1 信用、advanced 2 信用',
      ],
    },
    ui: {
      enabled: false, // 默认不启用：免费额度（但是需要登录 Tavily 账号获取token才能使用）
      canDisable: true,
      icon: 'i-carbon-search-locate text-lg text-orange-600 dark:text-orange-400',
      iconBg: 'bg-orange-100',
      darkIconBg: 'dark:bg-orange-900',
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

  // 1. 核心规范
  parts.push(CORE_RULES)

  // 2. 基础规范（紧凑连接到核心规范）
  const baseParts = enabledTools
    .map(t => t.prompt.base)
    .filter(Boolean)
    .map(b => `- ${b}`)

  if (baseParts.length > 0)
    parts[0] = `${parts[0]}\n${baseParts.join('\n')}`

  // 3. 工具使用细节（按工具分组，结构化输出）
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
    parts.push('## 工具使用指南\n')
    parts.push(toolDetails.join('\n\n'))
  }

  return parts.join('\n\n')
}

/**
 * 生成完整提示词（兼容旧版 MCPToolConfig 接口）
 * @param mcpTools 旧版工具配置列表
 * @returns 格式化的完整提示词
 */
export function generateFullPrompt(mcpTools: MCPToolConfig[]): string {
  // 将旧版配置映射到新版配置
  const toolsWithPrompt: ToolPromptConfig[] = []

  for (const tool of mcpTools) {
    // eslint-disable-next-line ts/ban-ts-comment
    // @ts-expect-error
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

// ----------------------------------------------------------------------------
// 向后兼容导出
// ----------------------------------------------------------------------------

/**
 * 从新配置派生的旧版 PROMPT_SECTIONS
 * 保持向后兼容性
 */
export const PROMPT_SECTIONS: Record<string, PromptSection> = MCP_TOOLS_CONFIG.reduce(
  (acc, tool) => {
    const { whenToUse, howToUse } = tool.prompt

    // 构建 detail 字符串
    const detailParts: string[] = [
      ...whenToUse.map(s => `- ${s}`),
      ...howToUse.map(s => `- ${s}`),
    ]

    acc[tool.id] = {
      base: tool.prompt.base ? `- ${tool.prompt.base}` : '',
      detail: detailParts.length > 0
        ? `${tool.name}工具使用细节：\n${detailParts.join('\n')}`
        : '',
    }
    return acc
  },
  {} as Record<string, PromptSection>,
)

/**
 * 从新配置派生的旧版 DEFAULT_MCP_TOOLS
 * 保持向后兼容性
 */
export const DEFAULT_MCP_TOOLS: MCPToolConfig[] = MCP_TOOLS_CONFIG.map(tool => ({
  id: tool.id,
  name: tool.name,
  description: tool.description,
  enabled: tool.ui.enabled,
  canDisable: tool.ui.canDisable,
  icon: tool.ui.icon,
  iconBg: tool.ui.iconBg,
  darkIconBg: tool.ui.darkIconBg,
}))

/**
 * 默认的完整提示词
 * 使用默认工具配置生成
 */
export const REFERENCE_PROMPT = generateFullPromptFromConfig(MCP_TOOLS_CONFIG)
