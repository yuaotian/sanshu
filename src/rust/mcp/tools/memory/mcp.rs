use anyhow::Result;
use rmcp::model::{ErrorData as McpError, CallToolResult, Content};

use super::{MemoryManager, MemoryCategory};
use crate::mcp::{JiyiRequest, utils::{validate_project_path, project_path_error}};
use crate::{log_debug, log_important};

/// 全局记忆管理工具
///
/// 用于存储和管理重要的开发规范、用户偏好和最佳实践
#[derive(Clone)]
pub struct MemoryTool;

impl MemoryTool {
    pub async fn jiyi(
        request: JiyiRequest,
    ) -> Result<CallToolResult, McpError> {
        log_important!(info, "[ji] 调用开始: action={}, project_path={}, content_len={}",
            request.action,
            request.project_path,
            request.content.len()
        );

        // 使用增强的路径验证功能
        if let Err(e) = validate_project_path(&request.project_path) {
            log_important!(warn, "[ji] 路径验证失败: {}", e);
            return Err(project_path_error(format!(
                "路径验证失败: {}\n原始路径: {}\n请检查路径格式是否正确，特别是 Windows 路径应使用正确的盘符格式（如 C:\\path）",
                e,
                request.project_path
            )).into());
        }

        // 创建记忆管理器（会自动执行迁移和启动时去重）
        // 支持非 Git 项目降级模式
        let start = std::time::Instant::now();
        let mut manager = MemoryManager::new(&request.project_path)
            .map_err(|e| {
                log_important!(error, "[ji] 创建记忆管理器失败: {}", e);
                McpError::internal_error(format!("创建记忆管理器失败: {}", e), None)
            })?;
        log_debug!("[ji] 记忆管理器创建完成: elapsed={}ms, is_non_git={}", 
            start.elapsed().as_millis(), manager.is_non_git_project());

        // 非 Git 项目提示（仅在降级模式时显示）
        let non_git_hint = if manager.is_non_git_project() {
            "\n\n⚠️ 当前目录非 Git 仓库，记忆已存储在项目根目录 `.sanshu-memory` 文件夹中。\n💡 建议初始化 Git 以获得更好的项目记忆隔离：`git init`"
        } else {
            ""
        };

        // 检查 sou 工具是否启用，如果启用则尝试触发后台索引
        let mut index_hint = String::new();
        if is_sou_enabled() {
            if let Err(e) = try_trigger_background_index(&request.project_path).await {
                log_debug!("触发后台索引失败（不影响记忆操作）: {}", e);
            } else {
                index_hint = "\n\n💡 已为当前项目后台启动代码索引，以便后续 sou 工具使用。".to_string();
            }
        }

        let result = match request.action.as_str() {
            "记忆" => {
                if request.content.trim().is_empty() {
                    log_important!(warn, "[ji] 记忆操作失败: 内容为空");
                    return Err(McpError::invalid_params("缺少记忆内容".to_string(), None));
                }

                // 使用 MemoryCategory 的新方法解析分类
                let category = MemoryCategory::from_str(&request.category);
                log_debug!("[ji] 执行记忆操作: category={:?}, content_len={}", category, request.content.len());

                // 添加记忆（带去重检测）
                match manager.add_memory(&request.content, category) {
                    Ok(Some(id)) => {
                        log_important!(info, "[ji] 记忆添加成功: id={}, category={:?}", id, category);
                        format!(
                            "✅ 记忆已添加，ID: {}\n📝 内容: {}\n📂 分类: {}{}{}",
                            id,
                            request.content,
                            category.display_name(),
                            index_hint,
                            non_git_hint
                        )
                    }
                    Ok(None) => {
                        // 被去重静默拒绝
                        log_debug!("[ji] 记忆被去重拒绝: 内容与现有记忆相似");
                        format!(
                            "⚠️ 记忆已存在相似内容，未重复添加\n📝 内容: {}\n📂 分类: {}{}{}",
                            request.content,
                            category.display_name(),
                            index_hint,
                            non_git_hint
                        )
                    }
                    Err(e) => {
                        log_important!(error, "[ji] 添加记忆失败: {}", e);
                        return Err(McpError::internal_error(format!("添加记忆失败: {}", e), None));
                    }
                }
            }
            "回忆" => {
                log_debug!("[ji] 执行回忆操作");
                let info = manager.get_project_info();
                log_important!(info, "[ji] 回忆完成: info_len={}", info.len());
                format!("{}{}{}", info, index_hint, non_git_hint)
            }
            // === 新增: 整理 (执行去重) ===
            "整理" => {
                log_debug!("[ji] 执行整理（去重）操作");
                match manager.deduplicate_with_stats() {
                    Ok(stats) => {
                        log_important!(info, "[ji] 去重完成: original={}, removed={}, remaining={}",
                            stats.original_count, stats.removed_count, stats.remaining_count);
                        // 返回 JSON 格式便于前端解析
                        let json_result = serde_json::json!({
                            "success": true,
                            "original_count": stats.original_count,
                            "removed_count": stats.removed_count,
                            "remaining_count": stats.remaining_count,
                            "removed_ids": stats.removed_ids
                        });
                        format!("✅ 去重整理完成\n{}", serde_json::to_string_pretty(&json_result).unwrap_or_default())
                    }
                    Err(e) => {
                        log_important!(error, "[ji] 去重整理失败: {}", e);
                        return Err(McpError::internal_error(format!("去重整理失败: {}", e), None));
                    }
                }
            }
            // === 新增: 列表 (获取全部记忆) ===
            "列表" => {
                log_debug!("[ji] 执行列表操作");
                let memories = manager.get_all_memories();
                let entries: Vec<serde_json::Value> = memories.iter().map(|m| {
                    serde_json::json!({
                        "id": m.id,
                        "content": m.content,
                        "category": m.category.display_name(),
                        "created_at": m.created_at.to_rfc3339()
                    })
                }).collect();
                
                let stats = manager.get_stats();
                log_important!(info, "[ji] 列表完成: total={}", stats.total);
                let json_result = serde_json::json!({
                    "total": stats.total,
                    "by_category": {
                        "规范": stats.rules,
                        "偏好": stats.preferences,
                        "模式": stats.patterns,
                        "背景": stats.contexts
                    },
                    "entries": entries
                });
                serde_json::to_string_pretty(&json_result).unwrap_or_else(|_| "[]".to_string())
            }
            // === 新增: 预览相似 (检测相似度) ===
            "预览相似" => {
                if request.content.trim().is_empty() {
                    log_important!(warn, "[ji] 预览相似失败: 内容为空");
                    return Err(McpError::invalid_params("缺少待检测内容".to_string(), None));
                }
                
                log_debug!("[ji] 执行预览相似: content_len={}", request.content.len());
                let dedup = super::dedup::MemoryDeduplicator::new(manager.config().similarity_threshold);
                let dup_info = dedup.check_duplicate(&request.content, &manager.get_all_memories().iter().map(|e| (*e).clone()).collect::<Vec<_>>());
                
                log_important!(info, "[ji] 相似度检测完成: is_dup={}, similarity={:.1}%", 
                    dup_info.is_duplicate, dup_info.similarity * 100.0);
                
                let json_result = serde_json::json!({
                    "is_duplicate": dup_info.is_duplicate,
                    "similarity": format!("{:.1}%", dup_info.similarity * 100.0),
                    "similarity_value": dup_info.similarity,
                    "threshold": manager.config().similarity_threshold,
                    "matched_id": dup_info.matched_id,
                    "matched_content": dup_info.matched_content
                });
                
                if dup_info.is_duplicate {
                    format!("⚠️ 检测到相似内容 (相似度: {:.1}%)\n{}", 
                        dup_info.similarity * 100.0,
                        serde_json::to_string_pretty(&json_result).unwrap_or_default())
                } else {
                    format!("✅ 未检测到相似内容 (最高相似度: {:.1}%)\n{}", 
                        dup_info.similarity * 100.0,
                        serde_json::to_string_pretty(&json_result).unwrap_or_default())
                }
            }
            // === 新增: 配置 (获取/更新配置) ===
            "配置" => {
                // 如果提供了 config 参数，则更新配置
                if let Some(config_req) = request.config {
                    log_debug!("[ji] 执行配置更新: {:?}", config_req);
                    let mut new_config = manager.config().clone();
                    
                    if let Some(threshold) = config_req.similarity_threshold {
                        // 验证阈值范围
                        new_config.similarity_threshold = threshold.clamp(0.5, 0.95);
                    }
                    if let Some(dedup_on_startup) = config_req.dedup_on_startup {
                        new_config.dedup_on_startup = dedup_on_startup;
                    }
                    if let Some(enable_dedup) = config_req.enable_dedup {
                        new_config.enable_dedup = enable_dedup;
                    }
                    
                    manager.update_config(new_config.clone())
                        .map_err(|e| {
                            log_important!(error, "[ji] 更新配置失败: {}", e);
                            McpError::internal_error(format!("更新配置失败: {}", e), None)
                        })?;
                    
                    log_important!(info, "[ji] 配置更新成功: threshold={}, dedup_on_startup={}, enable_dedup={}",
                        new_config.similarity_threshold, new_config.dedup_on_startup, new_config.enable_dedup);
                    
                    let json_result = serde_json::json!({
                        "success": true,
                        "message": "配置已更新",
                        "config": {
                            "similarity_threshold": new_config.similarity_threshold,
                            "dedup_on_startup": new_config.dedup_on_startup,
                            "enable_dedup": new_config.enable_dedup
                        }
                    });
                    format!("✅ 配置已更新\n{}", serde_json::to_string_pretty(&json_result).unwrap_or_default())
                } else {
                    // 返回当前配置
                    log_debug!("[ji] 获取当前配置");
                    let config = manager.config();
                    let json_result = serde_json::json!({
                        "similarity_threshold": config.similarity_threshold,
                        "dedup_on_startup": config.dedup_on_startup,
                        "enable_dedup": config.enable_dedup
                    });
                    format!("📋 当前配置\n{}", serde_json::to_string_pretty(&json_result).unwrap_or_default())
                }
            }
            // === 新增: 删除 (移除指定记忆) ===
            "删除" => {
                let memory_id = request.memory_id.as_deref()
                    .ok_or_else(|| {
                        log_important!(warn, "[ji] 删除失败: 缺少 memory_id");
                        McpError::invalid_params("缺少 memory_id 参数".to_string(), None)
                    })?;
                
                log_debug!("[ji] 执行删除操作: memory_id={}", memory_id);
                match manager.delete_memory(memory_id) {
                    Ok(Some(content)) => {
                        log_important!(info, "[ji] 删除成功: id={}, content_len={}", memory_id, content.len());
                        format!("✅ 已删除记忆\n🆔 ID: {}\n📝 内容: {}", memory_id, content)
                    }
                    Ok(None) => {
                        log_debug!("[ji] 删除失败: 未找到记忆 id={}", memory_id);
                        format!("⚠️ 未找到指定 ID 的记忆: {}", memory_id)
                    }
                    Err(e) => {
                        log_important!(error, "[ji] 删除记忆失败: {}", e);
                        return Err(McpError::internal_error(format!("删除记忆失败: {}", e), None));
                    }
                }
            }
            _ => {
                log_important!(warn, "[ji] 未知操作类型: {}", request.action);
                return Err(McpError::invalid_params(
                    format!("未知的操作类型: {}。支持的操作: 记忆 | 回忆 | 整理 | 列表 | 预览相似 | 配置 | 删除", request.action),
                    None
                ));
            }
        };

        log_important!(info, "[ji] 调用完成: action={}, result_len={}", request.action, result.len());
        Ok(CallToolResult::success(vec![Content::text(result)]))
    }
}

/// 检查 sou 工具是否启用
fn is_sou_enabled() -> bool {
    match crate::config::load_standalone_config() {
        Ok(config) => config.mcp_config.tools.get("sou").copied().unwrap_or(false),
        Err(_) => false,
    }
}

/// 尝试触发后台索引（仅在项目未初始化或索引失败时）
async fn try_trigger_background_index(project_root: &str) -> Result<()> {
    use super::super::acemcp::mcp::{get_initial_index_state, ensure_initial_index_background, InitialIndexState};

    // 获取 acemcp 配置：复用工具内部读取逻辑，避免字段新增/演进导致此处漏填
    let acemcp_config = super::super::acemcp::mcp::AcemcpTool::get_acemcp_config().await?;

    // 检查索引状态
    let initial_state = get_initial_index_state(project_root);

    // 仅在未初始化或失败时触发
    if matches!(initial_state, InitialIndexState::Missing | InitialIndexState::Idle | InitialIndexState::Failed) {
        ensure_initial_index_background(&acemcp_config, project_root).await?;
        Ok(())
    } else {
        // 已经完成或正在进行，无需操作
        Ok(())
    }
}
