use anyhow::Result;
use serde::Serialize;
// use tauri::{AppHandle, Emitter}; // 暂时不需要，由调用方处理事件
use teloxide::{
    prelude::*,
    types::{
        ChatId, InlineKeyboardButton, InlineKeyboardMarkup, KeyboardButton, KeyboardMarkup,
        MessageId, ParseMode,
    },
    Bot,
};

use super::markdown::process_telegram_markdown;
use crate::{log_important, log_debug};

/// Telegram事件类型
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum TelegramEvent {
    /// 选项状态变化
    OptionToggled { option: String, selected: bool },
    /// 文本输入更新
    TextUpdated { text: String },
    /// 继续按钮点击
    ContinuePressed,
    /// 发送按钮点击
    SendPressed,
}

/// Telegram Bot 核心功能
pub struct TelegramCore {
    pub bot: Bot,
    pub chat_id: ChatId,
}

impl TelegramCore {
    /// 创建新的Telegram核心实例
    pub fn new(bot_token: String, chat_id: String) -> Result<Self> {
        Self::new_with_api_url(bot_token, chat_id, None)
    }

    /// 创建新的Telegram核心实例，支持自定义API URL
    pub fn new_with_api_url(bot_token: String, chat_id: String, api_url: Option<String>) -> Result<Self> {
        // 日志：脱敏显示 token（只显示前后4位）
        let token_masked = if bot_token.len() > 8 {
            format!("{}...{}", &bot_token[..4], &bot_token[bot_token.len()-4..])
        } else {
            "****".to_string()
        };
        log_debug!("[telegram] 创建 TelegramCore: token={}, chat_id={}, custom_api={}", 
            token_masked, chat_id, api_url.is_some());

        let mut bot = Bot::new(bot_token);

        // 如果提供了自定义API URL，则设置它
        if let Some(url_str) = api_url {
            log_debug!("[telegram] 使用自定义 API URL: {}", url_str);
            let url = reqwest::Url::parse(&url_str)
                .map_err(|e| anyhow::anyhow!("无效的API URL格式: {}", e))?;
            bot = bot.set_api_url(url);
        }

        // 解析chat_id
        let chat_id = if chat_id.starts_with('@') {
            return Err(anyhow::anyhow!("暂不支持@username格式，请使用数字Chat ID"));
        } else {
            let id = chat_id
                .parse::<i64>()
                .map_err(|_| anyhow::anyhow!("无效的Chat ID格式，请使用数字ID"))?;
            ChatId(id)
        };

        log_important!(info, "[telegram] TelegramCore 创建成功: chat_id={}", chat_id.0);
        Ok(Self { bot, chat_id })
    }

    /// 发送普通消息
    pub async fn send_message(&self, message: &str) -> Result<()> {
        self.send_message_with_markdown(message, false).await
    }

    /// 发送支持Markdown的消息
    pub async fn send_message_with_markdown(
        &self,
        message: &str,
        use_markdown: bool,
    ) -> Result<()> {
        let msg_preview = if message.len() > 100 {
            format!("{}...(len={})", &message[..100], message.len())
        } else {
            message.to_string()
        };
        log_debug!("[telegram] 发送消息: markdown={}, preview={}", use_markdown, msg_preview);

        let mut send_request = self.bot.send_message(self.chat_id, message);

        // 如果启用Markdown，设置解析模式
        if use_markdown {
            send_request = send_request.parse_mode(ParseMode::MarkdownV2);
        }

        let start = std::time::Instant::now();
        send_request
            .await
            .map_err(|e| {
                log_important!(error, "[telegram] 发送消息失败: {}", e);
                anyhow::anyhow!("发送消息失败: {}", e)
            })?;

        log_debug!("[telegram] 消息发送成功: elapsed={}ms", start.elapsed().as_millis());
        Ok(())
    }

    /// 发送选项消息（消息一）
    pub async fn send_options_message(
        &self,
        message: &str,
        predefined_options: &[String],
        is_markdown: bool,
    ) -> Result<()> {
        let msg_len = message.len();
        let options_count = predefined_options.len();
        log_important!(info, "[telegram] 发送选项消息: msg_len={}, options_count={}, markdown={}", 
            msg_len, options_count, is_markdown);

        // 处理消息内容
        let processed_message = if is_markdown {
            process_telegram_markdown(message)
        } else {
            message.to_string()
        };

        // 创建消息发送请求
        let mut send_request = self.bot.send_message(self.chat_id, processed_message);

        // 只有当有预定义选项时才添加inline keyboard
        if !predefined_options.is_empty() {
            log_debug!("[telegram] 创建 inline keyboard: options={:?}", predefined_options);
            let inline_keyboard = Self::create_inline_keyboard(predefined_options, &[])?;
            send_request = send_request.reply_markup(inline_keyboard);
        }

        // 如果是Markdown，设置解析模式
        if is_markdown {
            send_request = send_request.parse_mode(ParseMode::MarkdownV2);
        }

        let start = std::time::Instant::now();
        match send_request.await {
            Ok(_) => {
                log_important!(info, "[telegram] 选项消息发送成功: elapsed={}ms", start.elapsed().as_millis());
                Ok(())
            }
            Err(e) => {
                let error_str = e.to_string();

                // 检查是否是JSON解析错误但消息实际发送成功
                let has_parsing_json = error_str.contains("parsing JSON");
                let has_ok_true = error_str.contains("\\\"ok\\\":true");

                if has_parsing_json && has_ok_true {
                    // 消息实际发送成功
                    log_debug!("[telegram] 选项消息发送成功（忽略 JSON 解析警告）: elapsed={}ms", start.elapsed().as_millis());
                    Ok(())
                } else {
                    log_important!(error, "[telegram] 选项消息发送失败: {}", e);
                    Err(anyhow::anyhow!("发送选项消息失败: {}", e))
                }
            }
        }
    }

    /// 发送操作消息（消息二）
    pub async fn send_operation_message(&self, continue_reply_enabled: bool) -> Result<i32> {
        log_debug!("[telegram] 发送操作消息: continue_enabled={}", continue_reply_enabled);

        // 创建reply keyboard
        let reply_keyboard = Self::create_reply_keyboard(continue_reply_enabled);

        // 发送操作消息
        let operation_message = "键盘上选择操作完成对话";

        let start = std::time::Instant::now();
        match self
            .bot
            .send_message(self.chat_id, operation_message)
            .reply_markup(reply_keyboard)
            .await
        {
            Ok(msg) => {
                log_debug!("[telegram] 操作消息发送成功: msg_id={}, elapsed={}ms", msg.id.0, start.elapsed().as_millis());
                Ok(msg.id.0)
            }
            Err(e) => {
                let error_str = e.to_string();
                // 检查是否是JSON解析错误但消息实际发送成功
                if error_str.contains("parsing JSON") && error_str.contains("\\\"ok\\\":true") {
                    // 消息实际发送成功，返回默认ID
                    log_debug!("[telegram] 操作消息发送成功（忽略 JSON 解析警告）: elapsed={}ms", start.elapsed().as_millis());
                    Ok(0)
                } else {
                    log_important!(error, "[telegram] 操作消息发送失败: {}", e);
                    Err(anyhow::anyhow!("发送操作消息失败: {}", e))
                }
            }
        }
    }

    /// 创建inline keyboard
    pub fn create_inline_keyboard(
        predefined_options: &[String],
        selected_options: &[String],
    ) -> Result<InlineKeyboardMarkup> {
        let mut keyboard_rows = Vec::new();

        // 添加选项按钮（每行最多2个）
        for chunk in predefined_options.chunks(2) {
            let mut row = Vec::new();
            for option in chunk {
                let callback_data = format!("toggle:{}", option);
                // 根据选中状态显示按钮
                let button_text = if selected_options.contains(option) {
                    format!("✅ {}", option)
                } else {
                    option.to_string()
                };

                row.push(InlineKeyboardButton::callback(button_text, callback_data));
            }
            keyboard_rows.push(row);
        }

        let keyboard = InlineKeyboardMarkup::new(keyboard_rows);
        Ok(keyboard)
    }

    /// 创建reply keyboard
    pub fn create_reply_keyboard(continue_reply_enabled: bool) -> KeyboardMarkup {
        let mut keyboard_buttons = vec![KeyboardButton::new("↗️发送")];

        if continue_reply_enabled {
            keyboard_buttons.insert(0, KeyboardButton::new("⏩继续"));
        }

        KeyboardMarkup::new(vec![keyboard_buttons])
            .resize_keyboard()
            .one_time_keyboard()
    }

    /// 更新inline keyboard中的选项状态
    pub async fn update_inline_keyboard(
        &self,
        message_id: i32,
        predefined_options: &[String],
        selected_options: &[String],
    ) -> Result<()> {
        let new_keyboard = Self::create_inline_keyboard(predefined_options, selected_options)?;

        match self
            .bot
            .edit_message_reply_markup(self.chat_id, MessageId(message_id))
            .reply_markup(new_keyboard)
            .await
        {
            Ok(_) => Ok(()),
            Err(_) => {
                // 键盘更新失败通常不是致命错误，记录但不中断流程
                Ok(())
            }
        }
    }
}

/// 处理callback query的通用函数（不发送事件，由调用方处理）
pub async fn handle_callback_query(
    bot: &Bot,
    callback_query: &CallbackQuery,
    target_chat_id: ChatId,
) -> ResponseResult<Option<String>> {
    // 检查是否是目标聊天
    if let Some(message) = &callback_query.message {
        if message.chat().id != target_chat_id {
            return Ok(None);
        }
    }

    let mut toggled_option = None;

    if let Some(data) = &callback_query.data {
        if data.starts_with("toggle:") {
            let option = data.strip_prefix("toggle:").unwrap().to_string();
            toggled_option = Some(option);
        }
    }

    // 回答callback query
    bot.answer_callback_query(&callback_query.id).await?;

    Ok(toggled_option)
}

/// 处理文本消息的通用函数（不发送事件，由调用方处理）
pub async fn handle_text_message(
    message: &Message,
    target_chat_id: ChatId,
    operation_message_id: Option<i32>,
) -> ResponseResult<Option<TelegramEvent>> {
    // 检查是否是目标聊天
    if message.chat.id != target_chat_id {
        return Ok(None);
    }

    // 检查消息ID过滤
    if let Some(op_id) = operation_message_id {
        if message.id.0 <= op_id {
            return Ok(None);
        }
    }

    if let Some(text) = message.text() {
        let event = match text {
            "⏩继续" => TelegramEvent::ContinuePressed,
            "↗️发送" => TelegramEvent::SendPressed,
            _ => TelegramEvent::TextUpdated {
                text: text.to_string(),
            },
        };

        return Ok(Some(event));
    }

    Ok(None)
}

/// 生成统一的反馈消息
pub fn build_feedback_message(
    selected_options: &[String],
    user_input: &str,
    is_continue: bool,
) -> String {
    if is_continue {
        // 继续操作的反馈消息
        let continue_prompt = if let Ok(config) = crate::config::load_standalone_config() {
            config.reply_config.continue_prompt
        } else {
            "请按照最佳实践继续".to_string()
        };

        format!("✅ 发送成功！\n\n📝 选中的选项：\n• ⏩ {}", continue_prompt)
    } else {
        // 发送操作的反馈消息
        let mut feedback_message = "✅ 发送成功！\n\n📝 选中的选项：\n".to_string();

        if selected_options.is_empty() {
            feedback_message.push_str("• 无");
        } else {
            for opt in selected_options {
                feedback_message.push_str(&format!("• {}\n", opt));
            }
        }

        if !user_input.is_empty() {
            feedback_message.push_str(&format!("\n📝 补充说明：\n{}", user_input));
        }

        feedback_message
    }
}

/// 测试Telegram连接的通用函数
pub async fn test_telegram_connection(bot_token: &str, chat_id: &str) -> Result<String> {
    test_telegram_connection_with_api_url(bot_token, chat_id, None).await
}

/// 测试Telegram连接的通用函数，支持自定义API URL
pub async fn test_telegram_connection_with_api_url(
    bot_token: &str,
    chat_id: &str,
    api_url: Option<&str>
) -> Result<String> {
    // 日志：脱敏显示 token
    let token_masked = if bot_token.len() > 8 {
        format!("{}...{}", &bot_token[..4], &bot_token[bot_token.len()-4..])
    } else {
        "****".to_string()
    };
    log_important!(info, "[telegram] 测试连接: token={}, chat_id={}, custom_api={}", 
        token_masked, chat_id, api_url.is_some());

    if bot_token.trim().is_empty() {
        log_important!(warn, "[telegram] 测试连接失败: Bot Token 为空");
        return Err(anyhow::anyhow!("Bot Token不能为空"));
    }

    if chat_id.trim().is_empty() {
        log_important!(warn, "[telegram] 测试连接失败: Chat ID 为空");
        return Err(anyhow::anyhow!("Chat ID不能为空"));
    }

    // 创建Bot实例
    let mut bot = Bot::new(bot_token);

    // 如果提供了自定义API URL，则设置它
    if let Some(url_str) = api_url {
        log_debug!("[telegram] 测试使用自定义 API URL: {}", url_str);
        let url = reqwest::Url::parse(url_str)
            .map_err(|e| anyhow::anyhow!("无效的API URL格式: {}", e))?;
        bot = bot.set_api_url(url);
    }

    // 验证Chat ID格式
    let chat_id_parsed: i64 = chat_id
        .parse()
        .map_err(|_| anyhow::anyhow!("Chat ID格式无效，请输入有效的数字ID"))?;

    // 发送测试消息
    let test_message =
        "🤖 三术应用测试消息\n\n这是一条来自三术应用的测试消息，表示Telegram Bot配置成功！";

    let start = std::time::Instant::now();
    match bot.send_message(ChatId(chat_id_parsed), test_message).await {
        Ok(_) => {
            log_important!(info, "[telegram] 测试连接成功: elapsed={}ms", start.elapsed().as_millis());
            Ok("测试消息发送成功！Telegram Bot配置正确。".to_string())
        }
        Err(e) => {
            log_important!(error, "[telegram] 测试连接失败: {}, elapsed={}ms", e, start.elapsed().as_millis());
            Err(anyhow::anyhow!("发送测试消息失败: {}", e))
        }
    }
}
