// UI/UX 文案本地化
// 仅保留单工具 uiux 所需的简洁文案

use super::types::{UiuxAction, UiuxLang};

pub fn error_text(lang: UiuxLang, message: &str) -> String {
    match lang {
        UiuxLang::Zh => format!("发生错误: {}", message),
        UiuxLang::En => format!("Error: {}", message),
    }
}

pub fn success_summary(
    lang: UiuxLang,
    action: UiuxAction,
    has_project_context: bool,
    degraded: bool,
) -> String {
    let action_text = match (lang, action) {
        (UiuxLang::Zh, UiuxAction::Beautify) => "UI 美化提示词",
        (UiuxLang::Zh, UiuxAction::Describe) => "UI 描述提示词",
        (UiuxLang::Zh, UiuxAction::Audit) => "UI 审查提示词",
        (UiuxLang::Zh, UiuxAction::DesignSystem) => "设计系统提示词",
        (UiuxLang::En, UiuxAction::Beautify) => "UI beautify prompt",
        (UiuxLang::En, UiuxAction::Describe) => "UI description prompt",
        (UiuxLang::En, UiuxAction::Audit) => "UI audit prompt",
        (UiuxLang::En, UiuxAction::DesignSystem) => "design system prompt",
    };

    match lang {
        UiuxLang::Zh => {
            let mut text = format!("已生成 {}。", action_text);
            if has_project_context {
                text.push_str(" 已追加项目上下文。");
            }
            if degraded {
                text.push_str(" 当前已降级到本地 markdown 检索。");
            }
            text
        }
        UiuxLang::En => {
            let mut text = format!("{} generated.", action_text);
            if has_project_context {
                text.push_str(" Project context appended.");
            }
            if degraded {
                text.push_str(" Fallback switched to local markdown retrieval.");
            }
            text
        }
    }
}
