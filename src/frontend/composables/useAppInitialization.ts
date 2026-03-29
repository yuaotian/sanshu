import { getCurrentWindow } from '@tauri-apps/api/window'
import { ref } from 'vue'
import { useFontManager } from './useFontManager'
import { initMcpTools } from './useMcpTools'
import { useSettings } from './useSettings'

/**
 * 应用初始化组合式函数
 */
export function useAppInitialization(mcpHandler: ReturnType<typeof import('./useMcpHandler').useMcpHandler>) {
  const isInitializing = ref(true)
  const { loadFontConfig, loadFontOptions } = useFontManager()
  const settings = useSettings()
  const { checkMcpMode, setupMcpEventListener } = mcpHandler

  /**
   * 检查是否为首次启动
   */
  function checkFirstRun(): boolean {
    // 检查localStorage是否有初始化标记
    const hasInitialized = localStorage.getItem('app-initialized')
    return !hasInitialized
  }

  /**
   * 标记应用已初始化
   */
  function markAsInitialized() {
    localStorage.setItem('app-initialized', 'true')
  }

  /**
   * 初始化应用
   */
  async function initializeApp() {
    try {
      const isFirstRun = checkFirstRun()

      // 并行加载字体和检测模式（互不依赖）
      const [, modeResult] = await Promise.all([
        Promise.all([loadFontConfig(), loadFontOptions()]),
        checkMcpMode(),
      ])

      const { isMcp, mcpContent, isIconMode, iconParams } = modeResult

      if (isIconMode && iconParams) {
        mcpHandler.setIconMode(true, iconParams)
      }

      // 加载窗口设置（显示窗口前必须完成）
      await Promise.all([
        settings.loadWindowSettings(),
        settings.loadWindowConfig(),
      ])

      // 基础设置完成，立即显示窗口（后续初始化可后台进行）
      isInitializing.value = false
      await getCurrentWindow().show()

      // 以下为非阻塞初始化，窗口已可见
      await settings.setupWindowFocusListener()

      if (isMcp) {
        try {
          await settings.syncWindowStateFromBackend()
        }
        catch (error) {
          console.warn('MCP模式状态同步失败，继续初始化:', error)
        }
      }

      if (!isMcp && !isIconMode) {
        await initMcpTools()
        await setupMcpEventListener()
      }

      if (isFirstRun) {
        markAsInitialized()
      }

      return { isMcp, mcpContent, isIconMode }
    }
    catch (error) {
      console.error('应用初始化失败:', error)
      isInitializing.value = false
      await getCurrentWindow().show().catch(() => {})
      throw error
    }
  }

  return {
    isInitializing,
    initializeApp,
  }
}
