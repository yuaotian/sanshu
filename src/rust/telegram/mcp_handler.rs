use anyhow::Result;
use std::collections::HashSet;
use teloxide::prelude::*;

use crate::config::load_standalone_config;
use crate::mcp::types::{build_continue_response, build_send_response, PopupRequest};
use crate::telegram::{handle_callback_query, handle_text_message, TelegramCore, TelegramEvent};
use crate::{log_important, log_debug};

/// 处理纯Telegram模式的MCP请求（不启动GUI）
pub async fn handle_telegram_only_mcp_request(request_file: &str) -> Result<()> {
    log_important!(info, "[telegram-mcp] 处理纯 Telegram 模式请求: file={}", request_file);

    // 读取MCP请求文件
    let request_json = std::fs::read_to_string(request_file)?;
    let request: PopupRequest = serde_json::from_str(&request_json)?;

    log_debug!("[telegram-mcp] 请求解析成功: id={}, msg_len={}, options={:?}", 
        request.id, 
        request.message.len(),
        request.predefined_options.as_ref().map(|o| o.len()).unwrap_or(0)
    );

    // 加载完整配置
    let app_config = load_standalone_config()?;
    let telegram_config = &app_config.telegram_config;

    if !telegram_config.enabled {
        log_important!(warn, "[telegram-mcp] Telegram 未启用，无法处理请求");
        return Ok(());
    }

    if telegram_config.bot_token.trim().is_empty() || telegram_config.chat_id.trim().is_empty() {
        log_important!(warn, "[telegram-mcp] Telegram 配置不完整: token_empty={}, chat_id_empty={}",
            telegram_config.bot_token.trim().is_empty(),
            telegram_config.chat_id.trim().is_empty()
        );
        return Ok(());
    }

    // 创建Telegram核心实例，使用配置中的API URL
    let api_url = if telegram_config.api_base_url == crate::constants::telegram::API_BASE_URL {
        None
    } else {
        Some(telegram_config.api_base_url.clone())
    };

    let core = TelegramCore::new_with_api_url(
        telegram_config.bot_token.clone(),
        telegram_config.chat_id.clone(),
        api_url,
    )?;

    // 发送消息到Telegram
    let predefined_options = request.predefined_options.clone().unwrap_or_default();

    log_important!(info, "[telegram-mcp] 开始发送消息: options_count={}", predefined_options.len());

    // 发送选项消息
    core.send_options_message(&request.message, &predefined_options, request.is_markdown)
        .await?;

    // 短暂延迟确保消息顺序
    tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;

    // 发送操作消息（假设启用继续回复）
    core.send_operation_message(true).await?;

    log_important!(info, "[telegram-mcp] 消息发送完成，启动监听循环");

    // 启动消息监听循环
    start_telegram_mcp_listener(core, request, predefined_options).await
}

/// 启动Telegram MCP消息监听循环
async fn start_telegram_mcp_listener(
    core: TelegramCore,
    request: PopupRequest,
    predefined_options: Vec<String>,
) -> Result<()> {
    let mut offset = 0i32;
    let mut selected_options: HashSet<String> = HashSet::new();
    let mut user_input = String::new();
    let mut options_message_id: Option<i32> = None;
    let mut poll_count = 0u32;

    log_debug!("[telegram-mcp] 监听循环启动: request_id={}", request.id);

    // 获取当前最新的消息ID作为基准
    if let Ok(updates) = core.bot.get_updates().limit(10).await {
        if let Some(update) = updates.last() {
            offset = update.id.0 as i32 + 1;
            log_debug!("[telegram-mcp] 获取初始 offset: {}", offset);
        }
    }

    // 监听循环（简化版本，只等待发送或继续操作）
    loop {
        poll_count += 1;
        if poll_count % 30 == 0 {
            // 每 30 次轮询（约 30 秒）记录一次心跳日志
            log_debug!("[telegram-mcp] 监听心跳: poll_count={}, offset={}, selected={}", 
                poll_count, offset, selected_options.len());
        }

        match core.bot.get_updates().offset(offset).timeout(10).await {
            Ok(updates) => {
                for update in updates {
                    offset = update.id.0 as i32 + 1;

                    match update.kind {
                        teloxide::types::UpdateKind::CallbackQuery(callback_query) => {
                            log_debug!("[telegram-mcp] 收到 CallbackQuery");
                            if let Err(e) = handle_callback_query_update(
                                &core,
                                &callback_query,
                                &predefined_options,
                                &mut selected_options,
                                &mut options_message_id,
                            ).await {
                                log_important!(warn, "[telegram-mcp] 处理 callback query 失败: {}", e);
                            }
                        }
                        teloxide::types::UpdateKind::Message(message) => {
                            log_debug!("[telegram-mcp] 收到 Message: text={:?}", message.text());
                            // 处理选项消息ID识别
                            if let Err(e) = handle_message_update(
                                &core,
                                &message,
                                &predefined_options,
                                &mut options_message_id,
                                &mut user_input,
                                &selected_options,
                                &request,
                            ).await {
                                if let Some(_result) = e.downcast_ref::<ProcessingComplete>() {
                                    log_important!(info, "[telegram-mcp] 监听完成，退出循环");
                                    return Ok(());
                                }
                                log_important!(warn, "[telegram-mcp] 处理消息失败: {}", e);
                            }
                        }
                        _ => {}
                    }
                }
            }
            Err(e) => {
                log_debug!("[telegram-mcp] 获取更新失败（将重试）: {}", e);
                tokio::time::sleep(tokio::time::Duration::from_secs(5)).await;
            }
        }

        // 短暂延迟避免过于频繁的请求
        tokio::time::sleep(tokio::time::Duration::from_millis(1000)).await;
    }
}

/// 处理callback query更新
async fn handle_callback_query_update(
    core: &TelegramCore,
    callback_query: &teloxide::types::CallbackQuery,
    predefined_options: &[String],
    selected_options: &mut HashSet<String>,
    options_message_id: &mut Option<i32>,
) -> Result<()> {
    // 只有当有预定义选项时才处理 callback queries
    if predefined_options.is_empty() {
        return Ok(());
    }

    // 从callback_query中提取消息ID
    if let Some(message) = &callback_query.message {
        if options_message_id.is_none() {
            *options_message_id = Some(message.id().0);
        }
    }

    if let Ok(Some(option)) = handle_callback_query(&core.bot, callback_query, core.chat_id).await {
        // 切换选项状态
        if selected_options.contains(&option) {
            selected_options.remove(&option);
        } else {
            selected_options.insert(option.clone());
        }

        // 更新按钮状态
        if let Some(msg_id) = *options_message_id {
            let selected_vec: Vec<String> = selected_options.iter().cloned().collect();
            let _ = core
                .update_inline_keyboard(msg_id, predefined_options, &selected_vec)
                .await;
        }
    }

    Ok(())
}

/// 处理消息更新
async fn handle_message_update(
    core: &TelegramCore,
    message: &teloxide::types::Message,
    predefined_options: &[String],
    options_message_id: &mut Option<i32>,
    user_input: &mut String,
    selected_options: &HashSet<String>,
    request: &PopupRequest,
) -> Result<()> {
    // 识别选项消息ID
    identify_options_message_id(message, predefined_options, options_message_id);

    // 处理文本消息事件
    if let Ok(Some(event)) = handle_text_message(message, core.chat_id, None).await {
        match event {
            TelegramEvent::SendPressed => {
                handle_send_pressed(core, selected_options, user_input, request).await?;
                return Err(ProcessingComplete.into());
            }
            TelegramEvent::ContinuePressed => {
                handle_continue_pressed(core, request).await?;
                return Err(ProcessingComplete.into());
            }
            TelegramEvent::TextUpdated { text } => {
                *user_input = text;
            }
            _ => {}
        }
    }

    Ok(())
}

/// 识别选项消息ID
fn identify_options_message_id(
    message: &teloxide::types::Message,
    predefined_options: &[String],
    options_message_id: &mut Option<i32>,
) {
    // 只有当有预定义选项时才检查 inline keyboard
    if predefined_options.is_empty() {
        return;
    }

    // 检查是否是包含 inline keyboard 的选项消息
    if let Some(inline_keyboard) = message.reply_markup() {
        // 检查是否包含我们的选项按钮
        let mut contains_our_options = false;
        for row in &inline_keyboard.inline_keyboard {
            for button in row {
                if let teloxide::types::InlineKeyboardButtonKind::CallbackData(callback_data) = &button.kind {
                    if callback_data.starts_with("toggle:") {
                        contains_our_options = true;
                        break;
                    }
                }
            }
            if contains_our_options {
                break;
            }
        }

        if contains_our_options {
            *options_message_id = Some(message.id.0);
        }
    }
}

/// 处理发送按钮按下
async fn handle_send_pressed(
    core: &TelegramCore,
    selected_options: &HashSet<String>,
    user_input: &str,
    request: &PopupRequest,
) -> Result<()> {
    log_important!(info, "[telegram-mcp] 用户点击发送: request_id={}, selected_count={}, input_len={}",
        request.id, selected_options.len(), user_input.len());

    // 使用统一的响应构建函数
    let selected_list: Vec<String> = selected_options.iter().cloned().collect();

    let user_input_option = if user_input.is_empty() {
        None
    } else {
        Some(user_input.to_string())
    };

    let response = build_send_response(
        user_input_option,
        selected_list.clone(),
        vec![], // 无GUI模式下没有图片
        Some(request.id.clone()),
        "telegram",
    );

    // 输出JSON响应到stdout（MCP协议要求）
    log_debug!("[telegram-mcp] 输出 MCP 响应: len={}", response.len());
    println!("{}", response);

    // 发送确认消息（使用统一的反馈消息生成函数）
    let feedback_message = crate::telegram::core::build_feedback_message(
        &selected_list,
        user_input,
        false, // 不是继续操作
    );
    let _ = core.send_message(&feedback_message).await;

    log_important!(info, "[telegram-mcp] 发送响应完成");
    Ok(())
}

/// 处理继续按钮按下
async fn handle_continue_pressed(
    core: &TelegramCore,
    request: &PopupRequest,
) -> Result<()> {
    log_important!(info, "[telegram-mcp] 用户点击继续: request_id={}", request.id);

    // 使用统一的继续响应构建函数
    let response = build_continue_response(
        Some(request.id.clone()),
        "telegram_continue",
    );

    // 输出JSON响应到stdout（MCP协议要求）
    log_debug!("[telegram-mcp] 输出继续响应: len={}", response.len());
    println!("{}", response);

    // 发送确认消息（使用统一的反馈消息生成函数）
    let feedback_message = crate::telegram::core::build_feedback_message(
        &[],  // 继续操作没有选项
        "",   // 继续操作没有用户输入
        true, // 是继续操作
    );
    let _ = core.send_message(&feedback_message).await;

    log_important!(info, "[telegram-mcp] 继续响应完成");
    Ok(())
}

/// 处理完成标记（用于从监听循环中退出）
#[derive(Debug)]
struct ProcessingComplete;

impl std::fmt::Display for ProcessingComplete {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Processing complete")
    }
}

impl std::error::Error for ProcessingComplete {}
