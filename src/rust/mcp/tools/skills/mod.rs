use std::borrow::Cow;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::Result;
use rmcp::model::{CallToolResult, Content, ErrorData as McpError, Tool};
use serde::{Deserialize, Serialize};
use tokio::process::Command;

use crate::config::load_standalone_config;
use crate::{log_debug, log_important};
use crate::mcp::types::SkillRunRequest;
use crate::mcp::tools::UiuxTool;

/// 技能运行时工具
/// 负责发现 skills、动态注册 MCP 工具并执行 Python 入口
pub struct SkillsTool;

#[derive(Debug, Clone)]
struct SkillInfo {
    name: String,
    description: Option<String>,
    path: PathBuf,
    config: Option<SkillConfig>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
struct SkillConfig {
    #[serde(default)]
    default_action: Option<String>,
    #[serde(default)]
    actions: Vec<SkillActionConfig>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
struct SkillActionConfig {
    name: String,
    entry: String,
    #[serde(default)]
    args_template: Option<Vec<String>>,
    #[serde(default)]
    allow_args: Option<bool>,
    #[serde(default)]
    description: Option<String>,
}

impl SkillsTool {
    /// 获取动态工具列表（包含 skill_run 与各个 skill_<name>）
    pub fn list_dynamic_tools(project_root: &Path) -> Vec<Tool> {
        let mut tools = Vec::new();
        tools.push(Self::get_skill_run_tool_definition());

        let skills = scan_skills(project_root);
        let input_schema = skills_input_schema();

        // 兼容 Antigravity：动态技能工具名使用下划线分隔
        for skill in skills {
            let tool_name = format!("skill_{}", skill.name);
            let description = skill.description.clone().unwrap_or_else(|| "技能工具".to_string());
            tools.push(Tool {
                name: Cow::Owned(tool_name),
                description: Some(Cow::Owned(description)),
                input_schema: Arc::new(input_schema.clone()),
                annotations: None,
                icons: None,
                meta: None,
                output_schema: None,
                title: None,
            });
        }

        tools
    }

    /// 处理技能工具调用
    pub async fn call_tool(
        tool_name: &str,
        mut request: SkillRunRequest,
        project_root: &Path,
    ) -> Result<CallToolResult, McpError> {
        let skill_name = if tool_name == "skill_run" {
            request.skill_name.clone().unwrap_or_default()
        } else {
            tool_name.trim_start_matches("skill_").to_string()
        };

        if skill_name.trim().is_empty() {
            return Err(McpError::invalid_params("缺少 skill_name".to_string(), None));
        }

        // 读取技能清单（按需加载，避免启动时全量解析）
        let skills = scan_skills(project_root);
        let skill = skills
            .into_iter()
            .find(|s| s.name.eq_ignore_ascii_case(&skill_name))
            .ok_or_else(|| McpError::invalid_params(format!("未找到技能: {}", skill_name), None))?;

        // 优先请求里的 action，其次使用配置默认 action，最后兜底 search
        let action_name = request
            .action
            .clone()
            .or_else(|| skill.config.as_ref().and_then(|c| c.default_action.clone()))
            .unwrap_or_else(|| "search".to_string());

        if skill.name == "ui-ux-pro-max" {
            return UiuxTool::call_from_skill(&action_name, &request).await;
        }

        let (entry_rel, args) = resolve_action_args(&skill, &action_name, &mut request)
            .map_err(|e| McpError::invalid_params(e.to_string(), None))?;

        // 构建入口路径，并限制在技能目录内执行（防止路径穿透）
        let entry_path = skill.path.join(&entry_rel);
        let entry_path = entry_path
            .canonicalize()
            .map_err(|e| McpError::invalid_params(format!("入口路径解析失败: {}", e), None))?;
        let skill_root = skill
            .path
            .canonicalize()
            .map_err(|e| McpError::invalid_params(format!("技能路径解析失败: {}", e), None))?;
        if !entry_path.starts_with(&skill_root) {
            return Err(McpError::invalid_params("入口路径不在技能目录内".to_string(), None));
        }

        // 选择 Python 执行器：配置优先，其次 PATH
        let python_bin = load_standalone_config()
            .ok()
            .and_then(|c| c.mcp_config.skill_python_path.clone())
            .unwrap_or_else(|| "python".to_string());

        log_important!(
            info,
            "技能执行: skill={}, action={}, entry={}, args={:?}",
            skill.name,
            action_name,
            entry_path.display(),
            args
        );

        let output = Command::new(python_bin)
            .arg(entry_path)
            .args(args)
            .current_dir(&skill.path)
            // 确保 Python 输出统一编码，避免控制台乱码
            .env("PYTHONIOENCODING", "utf-8")
            .output()
            .await
            .map_err(|e| McpError::internal_error(format!("Python 执行失败: {}", e), None))?;

        let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();

        if !output.status.success() {
            let err_text = if stderr.is_empty() { stdout } else { stderr };
            return Err(McpError::internal_error(format!("技能执行失败: {}", err_text), None));
        }

        let final_text = if stdout.is_empty() {
            "技能执行完成，但无输出".to_string()
        } else {
            stdout
        };

        Ok(CallToolResult::success(vec![Content::text(final_text)]))
    }

    fn get_skill_run_tool_definition() -> Tool {
        let schema = skills_input_schema();
        Tool {
            name: Cow::Borrowed("skill_run"),
            description: Some(Cow::Borrowed("通用技能执行工具，按名称调用指定 skill")),
            input_schema: Arc::new(schema),
            annotations: None,
            icons: None,
            meta: None,
            output_schema: None,
            title: None,
        }
    }
}

fn skills_input_schema() -> serde_json::Map<String, serde_json::Value> {
    let schema = serde_json::json!({
        "type": "object",
        "properties": {
            "skill_name": { "type": "string", "description": "技能名称（仅 skill_run 需要）" },
            "action": { "type": "string", "description": "动作名称（如 search/design_system/custom）" },
            "query": { "type": "string", "description": "查询或输入（可选）" },
            "args": { "type": "array", "items": { "type": "string" }, "description": "追加参数（可选）" }
        }
    });

    match schema {
        serde_json::Value::Object(map) => map,
        _ => serde_json::Map::new(),
    }
}

fn scan_skills(project_root: &Path) -> Vec<SkillInfo> {
    let mut skills_map: HashMap<String, SkillInfo> = HashMap::new();
    let mut seen_paths: HashSet<PathBuf> = HashSet::new();

    for root in build_skill_roots(project_root) {
        if !root.exists() {
            continue;
        }
        if seen_paths.contains(&root) {
            continue;
        }
        seen_paths.insert(root.clone());

        if let Ok(entries) = std::fs::read_dir(&root) {
            for entry in entries.flatten() {
                let path = entry.path();
                if !path.is_dir() {
                    continue;
                }
                let skill_md = path.join("SKILL.md");
                if !skill_md.exists() {
                    continue;
                }
                if let Ok(content) = std::fs::read_to_string(&skill_md) {
                    let (name_opt, desc_opt) = parse_skill_front_matter(&content);
                    let name = name_opt
                        .or_else(|| path.file_name().and_then(|s| s.to_str()).map(|s| s.to_string()))
                        .unwrap_or_else(|| "unknown-skill".to_string());
                    let normalized = normalize_skill_name(&name);
                    if skills_map.contains_key(&normalized) {
                        continue;
                    }

                    let config = load_skill_config(&path);
                    skills_map.insert(
                        normalized.clone(),
                        SkillInfo {
                            name: normalized,
                            description: desc_opt,
                            path: path.clone(),
                            config,
                        },
                    );
                }
            }
        }
    }

    let mut skills: Vec<SkillInfo> = skills_map.into_values().collect();
    skills.sort_by(|a, b| a.name.cmp(&b.name));
    skills
}

fn build_skill_roots(project_root: &Path) -> Vec<PathBuf> {
    let mut roots = Vec::new();

    // 项目内多生态目录
    let rel_paths = [
        ".codex/skills",
        ".claude/skills",
        ".continue/skills",
        ".opencode/skills",
        ".trae/skills",
        ".windsurf/skills",
        ".cursor/skills",
        ".gemini/skills",
        ".roo/skills",
        ".kiro/skills",
        ".qoder/skills",
        ".codebuddy/skills",
        ".agent/skills",
        ".shared/skills",
        "skills",
    ];

    for rel in rel_paths {
        roots.push(project_root.join(rel));
    }

    // 用户全局（Codex）
    if let Some(home) = dirs::home_dir() {
        roots.push(home.join(".codex").join("skills"));
    }

    roots
}

fn parse_skill_front_matter(content: &str) -> (Option<String>, Option<String>) {
    let mut lines = content.lines();
    let first = lines.next().unwrap_or("");
    if first.trim() != "---" {
        return (None, None);
    }

    let mut name: Option<String> = None;
    let mut description: Option<String> = None;

    for line in lines {
        let trimmed = line.trim();
        if trimmed == "---" || trimmed == "..." {
            break;
        }
        if let Some((key, value)) = trimmed.split_once(':') {
            let key = key.trim();
            let mut value = value.trim().to_string();
            value = strip_quotes(&value);
            match key {
                "name" => name = Some(value),
                "description" => description = Some(value),
                _ => {}
            }
        }
    }

    (name, description)
}

fn strip_quotes(input: &str) -> String {
    let s = input.trim();
    if (s.starts_with('"') && s.ends_with('"')) || (s.starts_with('\'') && s.ends_with('\'')) {
        s[1..s.len() - 1].to_string()
    } else {
        s.to_string()
    }
}

fn load_skill_config(skill_path: &Path) -> Option<SkillConfig> {
    let config_path = skill_path.join("skill.config.json");
    if !config_path.exists() {
        return None;
    }
    match std::fs::read_to_string(&config_path) {
        Ok(text) => match serde_json::from_str::<SkillConfig>(&text) {
            Ok(cfg) => Some(cfg),
            Err(e) => {
                log_debug!("解析 skill.config.json 失败: {}, path={}", e, config_path.display());
                None
            }
        },
        Err(e) => {
            log_debug!("读取 skill.config.json 失败: {}, path={}", e, config_path.display());
            None
        }
    }
}

fn resolve_action_args(
    skill: &SkillInfo,
    action_name: &str,
    request: &mut SkillRunRequest,
) -> Result<(String, Vec<String>)> {
    // 先尝试使用显式清单
    if let Some(config) = &skill.config {
        if let Some(action) = config.actions.iter().find(|a| a.name == action_name) {
            let mut args = Vec::new();
            if let Some(template) = &action.args_template {
                for token in template {
                    if token.contains("{query}") {
                        let q = request
                            .query
                            .clone()
                            .ok_or_else(|| anyhow::anyhow!("缺少 query 参数"))?;
                        args.push(token.replace("{query}", &q));
                    } else {
                        args.push(token.clone());
                    }
                }
            }
            if action.allow_args.unwrap_or(false) {
                if let Some(extra) = &request.args {
                    args.extend(extra.clone());
                }
            }

            return Ok((action.entry.clone(), args));
        }
    }

    // 兜底：约定式入口
    let search_entry = skill.path.join("scripts").join("search.py");
    let main_entry = skill.path.join("scripts").join("main.py");

    if search_entry.exists() {
        let mut args = Vec::new();
        if let Some(q) = &request.query {
            args.push(q.clone());
        }
        if let Some(extra) = &request.args {
            args.extend(extra.clone());
        }
        return Ok(("scripts/search.py".to_string(), args));
    }
    if main_entry.exists() {
        let mut args = Vec::new();
        if let Some(extra) = &request.args {
            args.extend(extra.clone());
        }
        return Ok(("scripts/main.py".to_string(), args));
    }

    log_debug!("技能未找到可执行入口: {}", skill.name);
    Err(anyhow::anyhow!("未找到可执行入口"))
}

fn normalize_skill_name(name: &str) -> String {
    // 使用小写与短横线，避免工具名包含空格或非法字符
    let mut out = String::new();
    for ch in name.chars() {
        if ch.is_ascii_alphanumeric() {
            out.push(ch.to_ascii_lowercase());
        } else if ch == '-' || ch == '_' {
            out.push('-');
        } else if ch.is_ascii_whitespace() {
            out.push('-');
        }
    }
    if out.is_empty() {
        "skill".to_string()
    } else {
        out
    }
}
