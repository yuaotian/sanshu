# sou fast-context 多后端接入总结

## 背景

`sou` 原先只接入 ACE / Augment Context Engine。当 ACE token 失效、索引未就绪或远端服务不可用时，`sou` 会无法继续提供代码上下文。本次改造在保留 ACE 能力的基础上，引入 `fast-context` 作为可配置后端，支持默认策略、主动切换、自动回退和双后端合并返回。

## 实现内容

- 新增 `SouTool` 路由层，统一承接 MCP `sou` 调用，并支持 `default`、`auto`、`ace`、`fast_context`、`both` 五种策略。
- 默认策略为 `auto`：优先调用 ACE，ACE 认证失败、配置缺失、索引未就绪或搜索失败时自动切换到 `fast-context`。
- fast-context 已迁移为 Rust 原生实现，不再依赖 `demo/fast-context-mcp-main` 中的 Node bridge。
- Rust 侧直接完成 Windsurf API Key 提取、JWT 获取、Connect-RPC/Protobuf 请求、本地受限命令执行和最终 `<ANSWER>` 解析。
- Rust 侧会把 fast-context 返回的文件与行号范围读取成本地代码片段，并格式化为兼容原 `Path: ...` 的 `sou` 输出；行号范围独立输出为 `Lines: Lx-Ly`，避免影响下游按扩展名识别路径。
- `both` 模式会同时返回 ACE 与 fast-context 结果；部分后端失败时保留诊断信息，避免整体搜索直接失败。
- `uiux` 已改为调用统一 `sou` 入口，继续兼容已有 `Path:` 片段解析逻辑。
- `save_acemcp_config` 对新增的 `sou_*` 与 `fast_context_*` 字段采用可选保留语义，避免提示词增强等旧配置页面保存 ACE 配置时清空后端策略。

## 配置页面

- `sou` 设置弹窗新增“后端切换”Tab。
- 可配置默认策略、auto 优先级、是否显示后端来源、是否保留失败诊断、Windsurf API Key、`tree_depth`、`max_turns`、`max_results`、超时时间和排除路径。
- “基础配置”中的 ACE 连接设置会根据默认策略动态提示；当默认策略不依赖 ACE 时，ACE URL 和 Token 输入会禁用并提示可留空。
- 调试区支持主动选择后端，便于验证 `default`、`auto`、`ace`、`fast_context`、`both` 的行为。

## 验证结果

- `cargo check` 通过。
- `pnpm build` 通过。
- `cargo test --test sou_fast_context_live --no-run` 通过，用于确认 live 验证测试可编译。
- `cargo test fast_context::tests` 通过，覆盖 Protobuf varint、Connect frame、工具调用解析、Connect error frame、ANSWER XML 与路径越界过滤。
- `SANSHU_LIVE_FAST_CONTEXT=1 cargo test --test sou_fast_context_live -- --ignored --nocapture` 通过，已完成一次真实 Windsurf fast-context smoke test。
- 相关改动文件无 linter 错误。

## 真实联通验证

仓库提供一个默认忽略的 live 集成测试：`tests/sou_fast_context_live.rs`。

运行前提：

- 本机已登录 Windsurf，或已设置 `WINDSURF_API_KEY`。
- 允许本次测试访问 Windsurf 远端 API，并可能消耗一次 fast-context 检索额度。

验证命令：

```bash
SANSHU_LIVE_FAST_CONTEXT=1 cargo test --test sou_fast_context_live -- --ignored --nocapture
```

Windows PowerShell：

```powershell
$env:SANSHU_LIVE_FAST_CONTEXT = "1"
cargo test --test sou_fast_context_live -- --ignored --nocapture
Remove-Item Env:SANSHU_LIVE_FAST_CONTEXT
```

测试会固定使用 `backend=fast_context`，并限制 `tree_depth=1`、`max_turns=1`、`max_results=3`，用于 smoke test，避免扩大请求范围。

## 注意事项

- 如未在设置中填写 Windsurf API Key，会读取 `WINDSURF_API_KEY` 或尝试从本机 Windsurf 安装中自动提取。
- 自动提取会读取 Windsurf 的 `state.vscdb` 中 `windsurfAuthStatus` 的 `apiKey` 字段：
  - macOS：`~/Library/Application Support/Windsurf/User/globalStorage/state.vscdb`
  - Windows：`%APPDATA%/Windsurf/User/globalStorage/state.vscdb`
  - Linux：`~/.config/Windsurf/User/globalStorage/state.vscdb`
- 读取 SQL：`SELECT value FROM ItemTable WHERE key = 'windsurfAuthStatus';`
- 本地受限命令会优先调用系统 PATH 中的 `rg`；如果不可用，会降级为 Rust 内置文件遍历和正则搜索。

## 原始需求

```

请你帮我完善`sou`，目前`sou`内置的是augment的ace，有时候token会失效导致无法使用，我现在需要你帮我再想办法接入`fast-context`，请你先了解`fast-context`和当前项目项目里面的`sou`实现，然后给我一个完整切换方案，除了可以配置主动切换、也可以选择默认、也可以选择一起返回的任意组合。

`代码搜索 配置` 的 页面设置呢？而且点开`sou`的mcp设置之后`基础配置`里面是不是也要动态调整？最核心的是还需要新增一个tab和可以配置主动切换、也可以选择默认、也可以选择一起返回的任意组合配置页面，默认ace返回如果失败了在主动切换到`fast-context`

demo\fast-context-mcp-main

src\rust\mcp\tools\acemcp

```
