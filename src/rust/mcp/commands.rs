use std::collections::HashMap;
use tauri::{AppHandle, State};

use crate::config::{AppState, save_config};
use crate::constants::mcp;
// use crate::mcp::tools::acemcp; // 已迁移到独立模块

/// MCP工具配置
#[derive(Debug, serde::Serialize, serde::Deserialize, Clone)]
pub struct MCPToolConfig {
    pub id: String,
    pub name: String,
    pub description: String,
    pub enabled: bool,
    pub can_disable: bool,
    pub icon: String,
    pub icon_bg: String,
    pub dark_icon_bg: String,
    pub has_config: bool, // 是否有配置选项
}

/// 获取MCP工具配置列表
#[tauri::command]
pub async fn get_mcp_tools_config(state: State<'_, AppState>) -> Result<Vec<MCPToolConfig>, String> {
    let config = state.config.lock().map_err(|e| format!("获取配置失败: {}", e))?;
    
    // 动态构建工具配置列表
    let mut tools = Vec::new();
    
    // 三术工具 - 始终存在，无配置选项
    tools.push(MCPToolConfig {
        id: mcp::TOOL_ZHI.to_string(),
        name: "三术".to_string(),
        description: "智能代码审查交互工具，支持预定义选项、自由文本输入和图片上传".to_string(),
        enabled: config.mcp_config.tools.get(mcp::TOOL_ZHI).copied().unwrap_or(true),
        can_disable: false, // 三术工具是必需的
        icon: "i-carbon-chat text-lg text-blue-600 dark:text-blue-400".to_string(),
        icon_bg: "bg-blue-100 dark:bg-blue-900".to_string(),
        dark_icon_bg: "dark:bg-blue-800".to_string(),
        has_config: false, // 三术工具没有配置选项
    });
    
    // 记忆管理工具 - 始终存在，无配置选项
    tools.push(MCPToolConfig {
        id: mcp::TOOL_JI.to_string(),
        name: "记忆管理".to_string(),
        description: "全局记忆管理工具，用于存储和管理重要的开发规范、用户偏好和最佳实践".to_string(),
        enabled: config.mcp_config.tools.get(mcp::TOOL_JI).copied().unwrap_or(true), // 修复：默认启用，与 default_mcp_tools() 保持一致
        can_disable: true,
        icon: "i-carbon-data-base text-lg text-purple-600 dark:text-purple-400".to_string(),
        icon_bg: "bg-green-100 dark:bg-green-900".to_string(),
        dark_icon_bg: "dark:bg-green-800".to_string(),
        has_config: false, // 记忆管理工具没有配置选项
    });
    
    // 代码搜索工具 - 始终存在，有配置选项
    tools.push(MCPToolConfig {
        id: mcp::TOOL_SOU.to_string(),
        name: "代码搜索".to_string(),
        description: "基于查询在特定项目中搜索相关的代码上下文，支持语义搜索和增量索引".to_string(),
        enabled: config.mcp_config.tools.get(mcp::TOOL_SOU).copied().unwrap_or(false),
        can_disable: true,
        icon: "i-carbon-search text-lg text-green-600 dark:text-green-400".to_string(),
        icon_bg: "bg-green-100 dark:bg-green-900".to_string(),
        dark_icon_bg: "dark:bg-green-800".to_string(),
        has_config: true, // 代码搜索工具有配置选项
    });

    // Context7 文档查询工具 - 始终存在，有配置选项
    tools.push(MCPToolConfig {
        id: mcp::TOOL_CONTEXT7.to_string(),
        name: "Context7 文档查询".to_string(),
        description: "查询最新的框架和库文档，支持 Next.js、React、Vue、Spring 等主流框架".to_string(),
        enabled: config.mcp_config.tools.get(mcp::TOOL_CONTEXT7).copied().unwrap_or(true),
        can_disable: true,
        icon: "i-carbon-document text-lg text-orange-600 dark:text-orange-400".to_string(),
        icon_bg: "bg-orange-100 dark:bg-orange-900".to_string(),
        dark_icon_bg: "dark:bg-orange-800".to_string(),
        has_config: true, // Context7 工具有配置选项
    });

    // UI/UX Pro Max 工具
    tools.push(MCPToolConfig {
        id: mcp::TOOL_UIUX.to_string(),
        name: "UI/UX Pro Max".to_string(),
        description: "UI/UX 设计智能检索与设计系统生成工具".to_string(),
        enabled: config.mcp_config.tools.get(mcp::TOOL_UIUX).copied().unwrap_or(true),
        can_disable: true,
        icon: "i-carbon-pen-fountain text-lg text-pink-600 dark:text-pink-400".to_string(),
        icon_bg: "bg-pink-100 dark:bg-pink-900".to_string(),
        dark_icon_bg: "dark:bg-pink-800".to_string(),
        has_config: false,
    });

    // 提示词增强工具 - 依赖 acemcp 配置
    tools.push(MCPToolConfig {
        id: mcp::TOOL_ENHANCE.to_string(),
        name: "提示词增强".to_string(),
        description: "将口语化提示词增强为结构化专业提示词，支持上下文与历史".to_string(),
        enabled: config.mcp_config.tools.get(mcp::TOOL_ENHANCE).copied().unwrap_or(false),
        can_disable: true,
        icon: "i-carbon-magic-wand text-lg text-indigo-600 dark:text-indigo-400".to_string(),
        icon_bg: "bg-indigo-100 dark:bg-indigo-900".to_string(),
        dark_icon_bg: "dark:bg-indigo-800".to_string(),
        has_config: false, // 提示词增强没有独立配置面板
    });

    // 图标工坊工具 - UI 功能工具，始终存在，有配置选项
    tools.push(MCPToolConfig {
        id: "icon".to_string(),
        name: "图标工坊".to_string(),
        description: "搜索和管理 Iconfont 图标库，支持预览、复制 SVG 和下载到项目".to_string(),
        enabled: config.mcp_config.tools.get("icon").copied().unwrap_or(true),
        can_disable: true,
        icon: "i-carbon-image text-lg text-purple-600 dark:text-purple-400".to_string(),
        icon_bg: "bg-purple-100 dark:bg-purple-900".to_string(),
        dark_icon_bg: "dark:bg-purple-800".to_string(),
        has_config: true, // 图标工坊有配置选项
    });

    // 按启用状态排序，启用的在前
    tools.sort_by(|a, b| b.enabled.cmp(&a.enabled));
    
    Ok(tools)
}

/// 设置MCP工具启用状态
#[tauri::command]
pub async fn set_mcp_tool_enabled(
    tool_id: String,
    enabled: bool,
    state: State<'_, AppState>,
    app: AppHandle,
) -> Result<(), String> {
    {
        let mut config = state.config.lock().map_err(|e| format!("获取配置失败: {}", e))?;
        
        // 检查工具是否可以禁用
        if tool_id == mcp::TOOL_ZHI && !enabled {
            return Err("三术工具是必需的，无法禁用".to_string());
        }
        
        // 更新工具状态
        config.mcp_config.tools.insert(tool_id.clone(), enabled);
    }
    
    // 保存配置
    save_config(&state, &app).await
        .map_err(|e| format!("保存配置失败: {}", e))?;

    // 使用日志记录状态变更（在 MCP 模式下会自动输出到文件）
    log::info!("MCP工具 {} 状态已更新为: {}", tool_id, enabled);

    Ok(())
}

/// 获取所有MCP工具状态
#[tauri::command]
pub async fn get_mcp_tools_status(state: State<'_, AppState>) -> Result<HashMap<String, bool>, String> {
    let config = state.config.lock().map_err(|e| format!("获取配置失败: {}", e))?;
    Ok(config.mcp_config.tools.clone())
}

/// 重置MCP工具配置为默认值
#[tauri::command]
pub async fn reset_mcp_tools_config(
    state: State<'_, AppState>,
    app: AppHandle,
) -> Result<(), String> {
    {
        let mut config = state.config.lock().map_err(|e| format!("获取配置失败: {}", e))?;
        let default_config = mcp::get_default_mcp_config();
        config.mcp_config.tools.clear();
        for tool in &default_config.tools {
            config.mcp_config.tools.insert(tool.tool_id.clone(), tool.enabled);
        }
    }
    
    // 保存配置
    save_config(&state, &app).await
        .map_err(|e| format!("保存配置失败: {}", e))?;

    // 使用日志记录配置重置（在 MCP 模式下会自动输出到文件）
    log::info!("MCP工具配置已重置为默认值");
    Ok(())
}

// acemcp 相关命令已迁移

// 已移除 Python Web 服务相关函数，完全使用 Rust 实现
// 如需调试配置，请直接查看本地配置文件
