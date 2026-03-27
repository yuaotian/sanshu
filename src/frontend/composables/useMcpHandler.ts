import { invoke } from '@tauri-apps/api/core'
import { listen } from '@tauri-apps/api/event'
import { ref } from 'vue'

/**
 * MCP处理组合式函数
 */
export function useMcpHandler() {
  const mcpRequest = ref(null)
  const showMcpPopup = ref(false)
  
  // 图标搜索模式状态
  const isIconMode = ref(false)
  const iconParams = ref<{
    query: string
    style: string
    savePath: string
    projectRoot: string
  } | null>(null)

  /**
   * 统一的MCP响应处理
   */
  async function handleMcpResponse(response: any) {
    try {
      await invoke('send_mcp_response', { response })
      await invoke('exit_app')
    }
    catch (error) {
      console.error('MCP响应处理失败:', error)
    }
  }

  /**
   * 统一的MCP取消处理
   */
  async function handleMcpCancel() {
    try {
      await invoke('send_mcp_response', { response: 'CANCELLED' })
      await invoke('exit_app')
    }
    catch (error) {
      console.error('MCP取消处理失败:', error)
    }
  }

  /**
   * 显示MCP弹窗
   */
  async function showMcpDialog(request: any) {
    // 获取Telegram配置，检查是否需要隐藏前端弹窗
    let shouldShowFrontendPopup = true
    try {
      const telegramConfig = await invoke('get_telegram_config')
      // 如果Telegram启用且配置了隐藏前端弹窗，则不显示前端弹窗
      if (telegramConfig && (telegramConfig as any).enabled && (telegramConfig as any).hide_frontend_popup) {
        shouldShowFrontendPopup = false
        console.log('🔕 根据Telegram配置，隐藏前端弹窗')
      }
    }
    catch (error) {
      console.error('获取Telegram配置失败:', error)
      // 配置获取失败时，保持默认行为（显示弹窗）
    }

    // 根据配置决定是否显示前端弹窗
    if (shouldShowFrontendPopup) {
      // 设置请求数据和显示状态
      mcpRequest.value = request
      showMcpPopup.value = true
    }
    else {
      console.log('🔕 跳过前端弹窗显示，仅使用Telegram交互')
    }

    // 播放音频通知（无论是否显示弹窗都播放）
    try {
      await invoke('play_notification_sound')
    }
    catch (error) {
      console.error('播放音频通知失败:', error)
    }

    // 启动Telegram同步（无论是否显示弹窗都启动）
    try {
      if (request?.message) {
        await invoke('start_telegram_sync', {
          message: request.message,
          predefinedOptions: request.predefined_options || [],
          isMarkdown: request.is_markdown || false,
        })
        console.log('✅ Telegram同步启动成功')
      }
    }
    catch (error) {
      console.error('启动Telegram同步失败:', error)
    }
  }

  /**
   * 检查MCP模式
   */
  async function checkMcpMode() {
    try {
      const args = await invoke('get_cli_args') as Record<string, any>

      // 检查是否为图标搜索模式
      if (args?.icon_mode) {
        console.log('📦 检测到图标搜索模式')
        return {
          isMcp: false,
          mcpContent: null,
          isIconMode: true,
          iconParams: {
            query: args.icon_query || '',
            style: args.icon_style || 'all',
            savePath: args.icon_save_path || 'assets/icons',
            projectRoot: args.icon_project_root || '',
          },
        }
      }

      // 检查是否为 MCP 请求模式
      if (args?.mcp_request) {
        // 读取MCP请求文件
        const content = await invoke('read_mcp_request', { filePath: args.mcp_request })

        if (content) {
          await showMcpDialog(content)
        }
        return { isMcp: true, mcpContent: content, isIconMode: false, iconParams: null }
      }

      // 检查是否为 CLI 交互模式
      if (args?.cli_request) {
        const content = args.cli_request
        if (content) {
          await showMcpDialog(content)
        }
        return { isMcp: true, mcpContent: content, isIconMode: false, iconParams: null }
      }
    }
    catch (error) {
      console.error('检查MCP模式失败:', error)
    }
    return { isMcp: false, mcpContent: null, isIconMode: false, iconParams: null }
  }

  /**
   * 设置MCP事件监听器
   */
  async function setupMcpEventListener() {
    try {
      await listen('mcp-request', (event) => {
        showMcpDialog(event.payload)
      })
    }
    catch (error) {
      console.error('设置MCP事件监听器失败:', error)
    }
  }
  /**
   * 设置图标模式状态
   */
  function setIconMode(mode: boolean, params: typeof iconParams.value = null) {
    isIconMode.value = mode
    iconParams.value = params
  }

  return {
    mcpRequest,
    showMcpPopup,
    isIconMode,
    iconParams,
    handleMcpResponse,
    handleMcpCancel,
    showMcpDialog,
    checkMcpMode,
    setupMcpEventListener,
    setIconMode,
  }
}
