import { invoke } from '@tauri-apps/api/core'
import { ref } from 'vue'

/**
 * 代理配置接口
 */
export interface ProxyConfig {
  auto_detect: boolean // 是否启用自动检测
  enabled: boolean // 是否启用代理（手动模式）
  proxy_type: string // 代理类型: "http" | "socks5"
  host: string // 代理主机地址
  port: number // 代理端口
  only_for_cn: boolean // 仅在中国大陆地区使用代理
}

/**
 * 代理信息接口
 */
export interface ProxyInfo {
  proxy_type: string
  host: string
  port: number
}

/**
 * 代理配置管理 Composable
 */
export function useProxyConfig() {
  const proxyConfig = ref<ProxyConfig>({
    auto_detect: true,
    enabled: false,
    proxy_type: 'http',
    host: '127.0.0.1',
    port: 7890,
    only_for_cn: true,
  })

  const isLoading = ref(false)
  const isTesting = ref(false)

  /**
   * 获取代理配置
   */
  async function getProxyConfig(): Promise<void> {
    try {
      isLoading.value = true
      const config = await invoke<ProxyConfig>('get_proxy_config')
      proxyConfig.value = config
    }
    catch (error) {
      console.error('获取代理配置失败:', error)
      throw error
    }
    finally {
      isLoading.value = false
    }
  }

  /**
   * 保存代理配置
   */
  async function saveProxyConfig(config?: ProxyConfig): Promise<void> {
    try {
      isLoading.value = true
      const configToSave = config || proxyConfig.value
      await invoke('set_proxy_config', { proxyConfig: configToSave })
      proxyConfig.value = configToSave
    }
    catch (error) {
      console.error('保存代理配置失败:', error)
      throw error
    }
    finally {
      isLoading.value = false
    }
  }

  /**
   * 测试代理连接
   */
  async function testProxyConnection(
    proxyType: string,
    host: string,
    port: number,
  ): Promise<boolean> {
    try {
      isTesting.value = true
      const result = await invoke<boolean>('test_proxy_connection', {
        proxyType,
        host,
        port,
      })
      return result
    }
    catch (error) {
      console.error('测试代理连接失败:', error)
      return false
    }
    finally {
      isTesting.value = false
    }
  }

  /**
   * 自动检测可用代理
   */
  async function detectAvailableProxy(): Promise<ProxyInfo | null> {
    try {
      isTesting.value = true
      const result = await invoke<ProxyInfo | null>('detect_available_proxy')
      return result
    }
    catch (error) {
      console.error('自动检测代理失败:', error)
      return null
    }
    finally {
      isTesting.value = false
    }
  }

  /**
   * 测试当前配置的代理
   */
  async function testCurrentProxy(): Promise<boolean> {
    return testProxyConnection(
      proxyConfig.value.proxy_type,
      proxyConfig.value.host,
      proxyConfig.value.port,
    )
  }

  return {
    proxyConfig,
    isLoading,
    isTesting,
    getProxyConfig,
    saveProxyConfig,
    testProxyConnection,
    detectAvailableProxy,
    testCurrentProxy,
  }
}
