use anyhow::Result;
use rmcp::model::{ErrorData as McpError, CallToolResult, Content};

use super::{MemoryManager, MemoryCategory};
use crate::mcp::{JiyiRequest, utils::{validate_project_path, project_path_error}};
use crate::log_debug;

/// 全局记忆管理工具
///
/// 用于存储和管理重要的开发规范、用户偏好和最佳实践
#[derive(Clone)]
pub struct MemoryTool;

impl MemoryTool {
    pub async fn jiyi(
        request: JiyiRequest,
    ) -> Result<CallToolResult, McpError> {
        // 使用增强的路径验证功能
        if let Err(e) = validate_project_path(&request.project_path) {
            return Err(project_path_error(format!(
                "路径验证失败: {}\n原始路径: {}\n请检查路径格式是否正确，特别是 Windows 路径应使用正确的盘符格式（如 C:\\path）",
                e,
                request.project_path
            )).into());
        }

        // 创建记忆管理器（会自动执行迁移和启动时去重）
        let mut manager = MemoryManager::new(&request.project_path)
            .map_err(|e| McpError::internal_error(format!("创建记忆管理器失败: {}", e), None))?;

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
                    return Err(McpError::invalid_params("缺少记忆内容".to_string(), None));
                }

                // 使用 MemoryCategory 的新方法解析分类
                let category = MemoryCategory::from_str(&request.category);

                // 添加记忆（带去重检测）
                match manager.add_memory(&request.content, category) {
                    Ok(Some(id)) => {
                        format!(
                            "✅ 记忆已添加，ID: {}\n📝 内容: {}\n📂 分类: {}{}",
                            id,
                            request.content,
                            category.display_name(),
                            index_hint
                        )
                    }
                    Ok(None) => {
                        // 被去重静默拒绝
                        format!(
                            "⚠️ 记忆已存在相似内容，未重复添加\n📝 内容: {}\n📂 分类: {}{}",
                            request.content,
                            category.display_name(),
                            index_hint
                        )
                    }
                    Err(e) => {
                        return Err(McpError::internal_error(format!("添加记忆失败: {}", e), None));
                    }
                }
            }
            "回忆" => {
                let info = manager.get_project_info();
                format!("{}{}", info, index_hint)
            }
            _ => {
                return Err(McpError::invalid_params(
                    format!("未知的操作类型: {}", request.action),
                    None
                ));
            }
        };

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
