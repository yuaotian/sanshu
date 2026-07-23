# UIUX Fast Context 知识检索优化总结

## 目标

- `uiux` 默认使用 `auto`：仅当 `sou` 已启用、后端策略包含 Fast Context、且本地检测到 API Key 时，才对 UI/UX 内置知识库执行 Fast Context 定向检索。
- Fast Context 无命中或请求失败时，回退到本地 Markdown 检索。
- 改善中文长查询、关键词拆分和知识片段分块，降低页脚与品牌词噪声。
- 支持请求级 `local` / `fast_context` 切换，便于使用完全相同的输入做 A/B 对比。
- 并行执行知识检索与项目上下文检索，并提供耗时、命中数和实际追加状态。
- 通过 `sou` 内部 typed sections 统一保留后端来源与精确行号。

## 实现

### 知识库物化

`src/rust/mcp/tools/uiux/knowledge_base.rs` 将编译期内嵌的
`ui-ux-pro-max-skill.md` 幂等写入系统配置目录下的
`sanshu/uiux-knowledge/`。内容未变化时不触碰文件，升级后内容变化时自动覆盖。

Fast Context 只检索该物化目录，并过滤掉知识库文件之外的返回结果。

### 后端选择

全局配置 `uiux_knowledge_backend` 支持：

- `auto`：默认值，完整检查 `sou` 开关、后端策略和本地 API Key 检测结果。
- `fast_context`：请求 Fast Context；本地未检测到 Key 或远端检索失败时回退本地。
- `local`：显式本地 A/B 基线，不标记为降级。

`uiux` 请求新增同名可选字段，可覆盖全局默认值：

```json
{
  "action": "audit",
  "query": "同一条 UI/UX 审查需求",
  "knowledge_backend": "local"
}
```

将 `knowledge_backend` 改为 `fast_context` 后再次调用，即可对比两次响应中的：

- `data.retrieval.knowledge_source`
- `data.retrieval.degraded`
- `data.retrieval.messages`
- `data.uiux_hits`
- `data.prompt`

API Key 的本地检测只说明 Key 已发现；实际鉴权、额度和服务状态以 Fast Context 检索结果为准。

全局默认值可在“代码搜索 → 后端切换 → UIUX 知识检索默认策略”中设置。请求级字段优先于该默认值。

### 并行与状态

知识检索和项目上下文检索使用独立 future 并行执行。响应额外提供：

- `requested_knowledge_backend`
- `project_context_appended`
- `knowledge_duration_ms` / `project_context_duration_ms`
- `knowledge_hit_count` / `project_context_hit_count`

`project_context_enabled` 表示用户请求了项目上下文，`project_context_appended` 表示实际命中并写入 prompt。项目检索失败或候选过滤为空时，响应会标记 `degraded=true`，摘要不会再误报“已追加项目上下文”。

### typed sou 片段

`sou` 增加 crate 内部结构化片段入口，统一解析 ACE 旧式 `Path (Lx-Ly)` 与 Fast Context `Path` + `Lines` 格式。`uiux` 直接消费 typed sections，不再重复解析 `CallToolResult` 文本；对外 `sou` MCP 文本协议保持不变。

### 本地中文检索

本地检索按 Markdown 标题分节，超长章节使用 24 行窗口和 4 行重叠。查询中的连续中文切为 bigram，ASCII 连续段按词处理，随后按内容命中率、标题命中率和字符相似度排序。

知识查询不再拼入文件名和 `UI/UX Pro Max` 品牌词，避免无关页脚或技术栈片段因固定词重复而排在前面。

## 验证

验证脚本：`scripts/test-uiux-knowledge-retrieval.ps1`

2026-07-24 最终本地执行结果：

- 本次涉及的 Rust 文件格式检查：通过。
- UIUX 模块单元测试：16 项通过，0 项失败。
- sou 结构化片段单元测试：4 项通过，0 项失败。
- `uiux_mcp` 集成测试：4 项通过，0 项失败。
- Rust library 编译检查：通过。
- `SouConfig.vue` ESLint：通过。
- 前端生产构建：通过；保留 1 条既有 IconWorkshop 动静态导入警告。

测试覆盖知识库幂等物化、旧内容修复、中文 bigram、标题分块、元章节/页脚噪声抑制、代表性查询 top-3、三种后端选择、项目上下文成功/失败状态、ACE/Fast Context 精确行号、请求 schema、观测字段和单工具输出契约。

## 运行边界

上述验证覆盖当前源码和本地检索路径。当前会话中的 MCP 服务进程需要重新加载新构建产物后，才会暴露请求级 `knowledge_backend` 字段；远端 A/B 结果还会受到所配置 Fast Context Key、额度与网络状态影响。
