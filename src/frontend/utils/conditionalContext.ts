import type { CustomPrompt, McpRequest } from '../types/popup'

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

/**
 * 获取 UI/UX 上下文策略状态信息（用于 UI 可视化展示）
 * @param request MCP 请求对象
 * @returns 策略状态信息，包含是否允许、图标、颜色、标签、原因等
 */
export function getContextPolicyStatus(request?: McpRequest | null): ContextPolicyStatus {
  const intent = request?.uiux_intent ?? 'none'
  const policy = request?.uiux_context_policy ?? 'auto'
  const reason = request?.uiux_reason

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
  } else if (isAutoBlocked) {
    // 自动策略下因无意图而阻止
    icon = 'i-carbon-warning'
    colorClass = 'text-yellow-400'
    label = '上下文未追加'
    generatedReason = reason || '当前无 UI/UX 相关意图，未追加条件性上下文'
  } else if (policy === 'force') {
    // 强制追加
    icon = 'i-carbon-checkmark-filled'
    colorClass = 'text-green-400'
    label = '上下文已追加'
    generatedReason = reason || `强制追加上下文（意图：${INTENT_LABELS[intent] || intent}）`
  } else {
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
 * @returns 当存在有效的 uiux_intent 或非 auto 策略时返回 true
 */
export function shouldShowPolicyIndicator(request?: McpRequest | null): boolean {
  if (!request) return false
  // 存在明确意图 或 策略非自动 或 存在原因说明
  return (
    (request.uiux_intent && request.uiux_intent !== 'none') ||
    (request.uiux_context_policy && request.uiux_context_policy !== 'auto') ||
    !!request.uiux_reason
  )
}

// 复用条件性 prompt 的上下文拼接逻辑，保持与弹窗输入一致
export function buildConditionalContext(prompts: CustomPrompt[], request?: McpRequest | null): string {
  const conditionalTexts: string[] = []

  // 根据 UI/UX 上下文策略决定是否追加条件性上下文
  const intent = request?.uiux_intent ?? 'none'
  const policy = request?.uiux_context_policy ?? 'auto'
  if (policy === 'forbid' || (policy === 'auto' && intent === 'none')) {
    return ''
  }

  prompts.forEach((prompt) => {
    const isEnabled = prompt.current_state ?? false
    const template = isEnabled ? prompt.template_true : prompt.template_false

    if (template && template.trim()) {
      conditionalTexts.push(template.trim())
    }
  })

  return conditionalTexts.join('\n')
}
