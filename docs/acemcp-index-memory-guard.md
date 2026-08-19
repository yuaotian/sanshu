# ACE 索引内存异常根因与范围保护

## 1. 结论

`三术.exe` 的高内存来自误将 `C:/Users/yuaotian` 当作项目根目录后，ACE 收集了百万级文件，并在上传前复制了多份包含文件全文的 `BlobItem` 集合。此次修复采用三层保护：关键路径拦截、统一生成目录排除、有界元数据预检；同时消除两次全量内容复制。

## 2. 现场证据

- 异常任务根目录：`C:/Users/yuaotian`。
- 日志记录扫描 `2,457,115` 个文件、索引 `1,119,026` 个文件、生成 `1,362,024` 个 blob。
- 任务共 `27,229` 批，只完成 2 批；旧进程会在 MCP 重启后继续恢复。
- 其他 28 个项目的最大 blob 数为 `2,536`，P95 为 `1,621`，与异常任务相差数百倍。
- 逐文件 INFO 日志令 `sanshu-mcp.log*` 一度累计约 1.4 GB。

## 3. 根因定位

- 全文收集：`src/rust/mcp/tools/acemcp/mcp.rs` 的 `collect_blobs`。
- 内存放大：`update_index_with_mode` 原先先保留 `blobs`，再完整 clone 为 `blob_entries`，又 clone 未完成项为 `new_entries`。
- 自动恢复：`resume_index_jobs` 会恢复 `queued/collecting/uploading/paused` 作业。
- 误选入口：`sou` 搜索会根据 `project_root_path` 自动启动 watcher 与后台索引；旧实现没有范围预检。

## 4. 已实施修复

### 4.1 三层项目范围保护

1. 精确命中文件系统根目录或当前用户主目录时立即暂停，不按盘符、`Documents` 名称等模糊条件判断。
2. 预检、实际收集与 watcher 共用 `effective_exclude_patterns`，覆盖 `node_modules`、`.git`、`target`、`dist`、`build`、`out`、`.next`、缓存和虚拟环境等生成目录。
3. 新路径只读取目录项和文件元数据，达到任一阈值即提前停止：
   - 目录项：`250,000`；
   - 候选源码/文本文件：`50,000`；
   - 候选文件总大小：`1 GiB`。

实现入口：`src/rust/mcp/tools/acemcp/scope_guard.rs` 的 `critical_path_risk`、`preflight_project_scope`；统一调度入口为 `src/rust/mcp/tools/acemcp/mcp.rs` 的 `ensure_project_scope_allowed`。

### 4.2 内存与日志

- `update_index_with_mode` 改为 `blobs.into_iter()`，并通过 `blob_entries.retain(...)` 原地删除已完成项，取消两次全量文件正文 clone。
- 每个文件的索引明细由重要 INFO 日志降为 DEBUG，仅保留任务级收集汇总。
- 本轮仍保留一次完整 `Vec<BlobItem>`，全链路流式上传属于后续 Plan B。

### 4.3 全局确认弹窗

- 风险诊断持久化在 `ProjectIndexStatus.scope_risk`，风险作业状态为 `scope_blocked`，不会被启动恢复逻辑继续执行。
- `src/frontend/components/AppContent.vue` 在窗口挂载和 MCP 请求变化时读取风险状态。
- `ProjectScopeRiskDialog.vue` 展示路径、原因、预检计数和项目标识，并提供：
  - `移除索引记录`：删除该路径的索引、状态、任务、持久监听和确认记录，不删除真实文件；
  - `确认这是项目`：持久化规范化路径；关键路径需要二次确认；
  - `暂不处理`：保留暂停状态，下次打开继续提示。

## 5. 运行态清理与部署

- 已停止仍执行异常作业的高内存旧进程，保留低内存 MCP 进程维持当前交互。
- 已精确删除 `C:/Users/yuaotian` 在 `index_jobs.json`、`projects.json`、`projects_status.json` 中的记录。
- 已从配置移除不存在的 `E:/ProjectCode/GoCode/sub2api/sub2api` 持久监听。
- 状态备份后缀：`pre-memory-guard-20260819-163952.bak`。
- Release `0.6.8` 已部署到 `D:/Mcp-Servers/sanshu`；旧可执行文件保留为同目录时间戳备份。

## 6. 验证与回滚

定向验证：

```powershell
./scripts/test-acemcp-scope-guard.ps1
```

已执行的检查包括范围保护单元测试、两个 Rust 二进制目标检查、定向 ESLint、Vite 生产构建和 Release 双二进制构建。

回滚时：先退出相关进程，再将 `D:/Mcp-Servers/sanshu/*.pre-memory-guard-*.bak` 恢复为原文件；运行态索引 JSON 可使用第 5 节记录的同后缀备份恢复。恢复百万级错误任务会重新触发原问题，因此通常只回滚二进制，不恢复该任务记录。

## 7. 后续 Plan B

把 `collect_blobs -> 排序 -> 上传` 改为磁盘检查点或有界批次的流式管线，使内存上限由项目总内容量降为单批内容量。该工作会改变排序、断点哈希和批次确认契约，需单独设计和回归，不并入本轮最小闭环。
