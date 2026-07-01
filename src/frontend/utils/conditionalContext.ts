import type { ContextScope, CustomPrompt, McpRequest, MemoryCategory, MemoryPolicy, ResponseContextBlock } from '../types/popup'

// UI/UX 上下文策略状态信息（用于 UI 可视化展示）
export interface ContextPolicyStatus {
  // 是否允许追加上下文
  allowed: boolean
  // 状态图标类名
  icon: string
  // 状态颜色（Tailwind 类名）
  colorClass: string
  // 状态标签文本
  label: string
  // 策略原因（来自 uiux_reason 或自动生成）
  reason: string
  // 原始意图值
  intent: 'none' | 'beautify' | 'page_refactor' | 'uiux_search'
  // 原始策略值
  policy: 'auto' | 'force' | 'forbid'
}

// 意图名称映射（用于 UI 展示）
const INTENT_LABELS: Record<string, string> = {
  none: '无特定意图',
  beautify: 'UI 美化',
  page_refactor: '页面重构',
  uiux_search: 'UI/UX 搜索',
}

// 策略名称映射（用于 UI 展示）
const POLICY_LABELS: Record<string, string> = {
  auto: '自动',
  force: '强制追加',
  forbid: '禁止追加',
}

const CONTEXT_SCOPE_LABELS: Record<ContextScope, string> = {
  turn: '本轮上下文',
  memory: '长期记忆',
  rule: '项目规则',
}

/**
 * 获取 UI/UX 上下文策略状态信息（用于 UI 可视化展示）
 * @param request MCP 请求对象
 * @returns 策略状态信息，包含是否允许、图标、颜色、标签、原因等
 */
export function getContextPolicyStatus(request?: McpRequest | null): ContextPolicyStatus {
  const intent = request?.uiux_intent ?? 'none'
  const policy = request?.uiux_context_policy ?? 'auto'
  const reason = request?.uiux_reason
  // 记录是否显式传入 UI/UX 上下文信号，便于区分默认与显式策略
  const hasExplicitSignal = !!(request?.uiux_intent || request?.uiux_context_policy || request?.uiux_reason)

  // 判断是否允许追加上下文
  const isForbidden = policy === 'forbid'
  const isAutoBlocked = policy === 'auto' && intent === 'none'
  const allowed = !isForbidden && !isAutoBlocked

  // 根据状态确定图标和颜色
  let icon: string
  let colorClass: string
  let label: string
  let generatedReason: string

  if (isForbidden) {
    // 策略明确禁止
    icon = 'i-carbon-close-outline'
    colorClass = 'text-red-400'
    label = '上下文已禁止'
    generatedReason = reason || '策略设置为禁止追加上下文'
  }
  else if (isAutoBlocked) {
    // 自动策略下因无意图而阻止
    icon = 'i-carbon-warning'
    colorClass = 'text-yellow-400'
    label = hasExplicitSignal ? '上下文未追加' : '上下文默认未追加'
    generatedReason = reason || (hasExplicitSignal
      ? '当前无 UI/UX 相关意图，未追加条件性上下文'
      : '未传入 UI/UX 上下文信号，按默认策略未追加')
  }
  else if (policy === 'force') {
    // 强制追加
    icon = 'i-carbon-checkmark-filled'
    colorClass = 'text-green-400'
    label = '上下文已追加'
    generatedReason = reason || `强制追加上下文（意图：${INTENT_LABELS[intent] || intent}）`
  }
  else {
    // 自动策略下允许追加（有意图）
    icon = 'i-carbon-checkmark'
    colorClass = 'text-blue-400'
    label = '上下文已追加'
    generatedReason = reason || `基于意图自动追加（${INTENT_LABELS[intent] || intent}）`
  }

  return {
    allowed,
    icon,
    colorClass,
    label,
    reason: generatedReason,
    intent,
    policy,
  }
}

/**
 * 判断是否应该显示策略指示器
 * @param request MCP 请求对象
 * @returns 只有显式传入 UI/UX 信号时才显示策略指示器（YAGNI：不显示用户不需要的信息）
 */
export function shouldShowPolicyIndicator(request?: McpRequest | null): boolean {
  if (!request)
    return false
  // 只有 AI 显式传入 UI/UX 参数时才显示策略指示器，避免非 UI 美化场景的无关提示
  return !!(request.uiux_intent || request.uiux_context_policy || request.uiux_reason)
}

function shouldAppendConditionalContext(request?: McpRequest | null): boolean {
  // 检查是否有显式 UI/UX 上下文信号
  const hasExplicitSignal = !!(request?.uiux_intent || request?.uiux_context_policy || request?.uiux_reason)

  // 只有当显式传入了 UI/UX 信号时，才应用策略检查
  // 否则（普通 zhi 调用），默认允许追加用户自定义的条件性上下文
  // 修复：此前默认 policy='auto' + intent='none' 会拦截所有未传信号的场景，
  // 导致用户勾选的上下文追加(条件性 Prompt)永远不会被拼接到最终响应中
  if (hasExplicitSignal) {
    const intent = request?.uiux_intent ?? 'none'
    const policy = request?.uiux_context_policy ?? 'auto'
    if (policy === 'forbid' || (policy === 'auto' && intent === 'none')) {
      return false
    }
  }

  return true
}

export function normalizeContextScope(scope?: string | null): ContextScope {
  if (scope === 'memory' || scope === 'rule')
    return scope
  return 'turn'
}

export function getContextScopeLabel(scope?: string | null): string {
  return CONTEXT_SCOPE_LABELS[normalizeContextScope(scope)]
}

export function getMemoryPolicy(scope?: string | null): MemoryPolicy {
  return normalizeContextScope(scope) === 'turn' ? 'never' : 'save'
}

export function getMemoryCategory(scope?: string | null): MemoryCategory | null {
  const normalizedScope = normalizeContextScope(scope)
  if (normalizedScope === 'memory')
    return 'preference'
  if (normalizedScope === 'rule')
    return 'rule'
  return null
}

export function sanitizeConditionalTemplate(template: string): string {
  return template
    .trim()
    .replace(/^[✔✅☑️☑\s]+/u, '')
    .replace(/^[❌✗✘🚫\s]+/u, '')
    .replace(/^请记住[，,:：\s]*/u, '')
    .trim()
}

// 生成结构化上下文块，避免把条件性 prompt 伪装成用户原始输入
export function buildConditionalContextBlocks(prompts: CustomPrompt[], request?: McpRequest | null): ResponseContextBlock[] {
  if (!shouldAppendConditionalContext(request))
    return []

  const blocks: ResponseContextBlock[] = []

  prompts.forEach((prompt) => {
    const isEnabled = prompt.current_state ?? false
    const template = isEnabled ? prompt.template_true : prompt.template_false

    if (template && template.trim()) {
      const content = sanitizeConditionalTemplate(template)
      if (!content)
        return

      const scope = normalizeContextScope(prompt.context_scope)
      blocks.push({
        kind: 'conditional_prompt',
        scope,
        memory_policy: getMemoryPolicy(scope),
        memory_category: getMemoryCategory(scope),
        content,
        source_id: prompt.id,
        source_name: prompt.name || prompt.condition_text || null,
      })
    }
  })

  return blocks
}

// 复用条件性 prompt 的上下文拼接逻辑，保持与本地增强等文本链路兼容
export function buildConditionalContext(prompts: CustomPrompt[], request?: McpRequest | null): string {
  return buildConditionalContextBlocks(prompts, request)
    .map(block => `[${getContextScopeLabel(block.scope)}] ${block.content}`)
    .join('\n')
}
