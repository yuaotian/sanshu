# UI/UX Pro Max 规则

来源：
- src/rust/assets/resources/ui-ux-pro-max-v2.15.0
- src/rust/mcp/tools/uiux/structured_search.rs

规则类别（按优先级）：
1. 可访问性（CRITICAL）
2. 触控与交互（CRITICAL）
3. 性能（HIGH）
4. 布局与响应式（HIGH）
5. 字体与色彩（MEDIUM）
6. 动效（MEDIUM）
7. 风格选择（MEDIUM）
8. 图表与数据（LOW）

关键规则要点（示例）：
- 可访问性：文本对比度至少 4.5:1；交互元素需有可见焦点态；图像需提供替代文本。
- 触控与交互：触控目标不小于 44x44px；主要交互使用点击/触控；异步按钮需禁用与提示。
- 性能：图片优先 WebP + 懒加载；动画需尊重 prefers-reduced-motion；异步内容预留空间避免跳动。
- 布局与响应式：移动端正文最小 16px；避免水平滚动；建立明确的 z-index 规则。
- 字体与色彩：正文行高 1.5-1.75；行宽控制 65-75 字符；标题/正文字体风格需匹配。
- 动效：微交互 150-300ms；优先 transform/opacity；加载态用骨架屏或 spinner。
- 风格选择：风格需匹配产品类型；全站一致；避免用 emoji 作为图标。
- 图表与数据：图表类型匹配数据形态；提供可访问色板与表格替代。

备注：当前正式知识基线为 `v2.15.0`。`uiux` 直接检索结构化 CSV，并按需物化同源 Markdown；旧单文件资源路径已移除。

## 触发与使用（MCP 版）

- 当用户请求涉及 **前端页面 / UI 设计 / 视觉美化 / 组件布局 / 设计系统** 等场景时，直接使用单一工具 `uiux`。
- `uiux` 使用 `sou` 检索项目内的真实页面/组件上下文；知识侧 `auto` 使用本地 v2.15.0 结构化 BM25 + BGE 混合检索，`local` 保留 BM25 基线，`fast_context` 仅作为显式对比入口。
- 推荐显式传入 `action=beautify|describe|audit|design_system`，默认 `beautify`。
- 保持用户控制：未经 `zhi` 明确确认，不自动执行 UI/UX 相关工具调用。
