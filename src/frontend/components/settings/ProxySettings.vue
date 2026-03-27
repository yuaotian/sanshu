<script setup lang="ts">
import { useMessage } from 'naive-ui'
import { onMounted, ref, watch } from 'vue'
import { useProxyConfig } from '../../composables/useProxyConfig'

const message = useMessage()
const {
  proxyConfig,
  isLoading,
  isTesting,
  isDetecting,
  getProxyConfig,
  saveProxyConfig,
  testCurrentProxy,
  testProxyConnection,
  detectAvailableProxy,
} = useProxyConfig()

// 本地编辑状态
const localConfig = ref({ ...proxyConfig.value })

// 代理类型选项
const proxyTypeOptions = [
  { label: 'HTTP/HTTPS', value: 'http' },
  { label: 'SOCKS5', value: 'socks5' },
]

// 常用端口预设
const commonPorts = [
  { label: 'Clash 混合端口', value: 7890 },
  { label: 'Clash HTTP 端口', value: 7891 },
  { label: 'V2Ray HTTP 端口', value: 10808 },
  { label: 'V2Ray SOCKS5 端口', value: 10809 },
  { label: '通用 SOCKS5 端口', value: 1080 },
  { label: '通用 HTTP 端口', value: 8080 },
]

// 加载配置
onMounted(async () => {
  try {
    await getProxyConfig()
    localConfig.value = { ...proxyConfig.value }
  }
  catch (error) {
    message.error(`加载代理配置失败: ${error}`)
  }
})

// 监听配置变化
watch(proxyConfig, (newConfig) => {
  localConfig.value = { ...newConfig }
}, { deep: true })

// 保存配置
async function handleSaveConfig() {
  try {
    await saveProxyConfig(localConfig.value)
    message.success('代理配置已保存')
  }
  catch (error) {
    message.error(`保存代理配置失败: ${error}`)
  }
}

// 测试代理连接
async function handleTestProxy() {
  try {
    const result = await testProxyConnection(
      localConfig.value.proxy_type,
      localConfig.value.host,
      localConfig.value.port,
    )
    if (result) {
      message.success('代理连接测试成功')
    }
    else {
      message.error('代理连接测试失败，请检查配置')
    }
  }
  catch (error) {
    message.error(`测试代理连接时出错: ${error}`)
  }
}

// 自动检测代理
async function handleAutoDetect() {
  try {
    const result = await detectAvailableProxy()
    if (result) {
      localConfig.value.proxy_type = result.proxy_type
      localConfig.value.host = result.host
      localConfig.value.port = result.port
      message.success(`检测到可用代理: ${result.proxy_type}://${result.host}:${result.port}`)
    }
    else {
      message.warning('未检测到可用的本地代理')
    }
  }
  catch (error) {
    message.error(`自动检测代理失败: ${error}`)
  }
}

// 应用预设端口
function applyPresetPort(port: number) {
  localConfig.value.port = port
  handleSaveConfig()
}
</script>

<template>
  <n-spin :show="isLoading">
    <n-space vertical size="large">
      <!-- 自动检测代理 -->
      <div class="flex items-center justify-between">
        <div class="flex items-center">
          <div class="w-1.5 h-1.5 bg-primary rounded-full mr-3 flex-shrink-0" />
          <div>
            <div class="text-sm font-medium leading-relaxed">
              自动检测代理
            </div>
            <div class="text-xs opacity-60">
              启用后，将根据地理位置自动检测并使用本地代理
            </div>
          </div>
        </div>
        <n-switch
          v-model:value="localConfig.auto_detect"
          size="small"
          @update:value="handleSaveConfig"
        />
      </div>

      <!-- 仅在中国大陆使用代理 -->
      <div v-if="localConfig.auto_detect" class="flex items-center justify-between">
        <div class="flex items-center">
          <div class="w-1.5 h-1.5 bg-info rounded-full mr-3 flex-shrink-0" />
          <div>
            <div class="text-sm font-medium leading-relaxed">
              仅在中国大陆使用代理
            </div>
            <div class="text-xs opacity-60">
              启用后，仅在检测到IP位于中国大陆时使用代理
            </div>
          </div>
        </div>
        <n-switch
          v-model:value="localConfig.only_for_cn"
          size="small"
          @update:value="handleSaveConfig"
        />
      </div>

      <!-- 手动代理配置 -->
      <div class="flex items-center justify-between">
        <div class="flex items-center">
          <div class="w-1.5 h-1.5 bg-warning rounded-full mr-3 flex-shrink-0" />
          <div>
            <div class="text-sm font-medium leading-relaxed">
              手动代理配置
            </div>
            <div class="text-xs opacity-60">
              启用后，将使用下方配置的代理（优先级低于自动检测）
            </div>
          </div>
        </div>
        <n-switch
          v-model:value="localConfig.enabled"
          size="small"
          :disabled="localConfig.auto_detect"
          @update:value="handleSaveConfig"
        />
      </div>

    <!-- 代理服务器配置 -->
    <div class="pt-4 border-t border-gray-200 dark:border-gray-700">
        <div class="flex items-start">
          <div class="w-1.5 h-1.5 bg-success rounded-full mr-3 mt-2 flex-shrink-0" />
          <div class="flex-1">
            <div class="text-sm font-medium mb-3 leading-relaxed">
              代理服务器配置
            </div>

            <n-form :model="localConfig" label-placement="left" label-width="auto" size="small">
              <n-form-item label="代理类型">
                <n-radio-group
                  v-model:value="localConfig.proxy_type"
                  @update:value="handleSaveConfig"
                >
                  <n-space>
                    <n-radio
                      v-for="option in proxyTypeOptions"
                      :key="option.value"
                      :value="option.value"
                    >
                      {{ option.label }}
                    </n-radio>
                  </n-space>
                </n-radio-group>
              </n-form-item>

              <n-form-item label="代理主机">
                <n-input
                  v-model:value="localConfig.host"
                  placeholder="127.0.0.1"
                  size="small"
                  @blur="handleSaveConfig"
                />
              </n-form-item>

              <n-form-item label="代理端口">
                <n-input-number
                  v-model:value="localConfig.port"
                  :min="1"
                  :max="65535"
                  placeholder="7890"
                  size="small"
                  class="w-full"
                  @blur="handleSaveConfig"
                />
              </n-form-item>

              <n-form-item label="端口预设">
                <n-space>
                  <n-button
                    v-for="preset in commonPorts"
                    :key="preset.value"
                    size="small"
                    @click="applyPresetPort(preset.value)"
                  >
                    {{ preset.label }} ({{ preset.value }})
                  </n-button>
                </n-space>
              </n-form-item>
            </n-form>

            <n-space>
              <n-button
                type="primary"
                size="small"
                :loading="isTesting"
                @click="handleTestProxy"
              >
                测试连接
              </n-button>
              <n-button
                size="small"
                :loading="isDetecting"
                @click="handleAutoDetect"
              >
                自动检测
              </n-button>
            </n-space>
          </div>
        </div>
      </div>

      <!-- 说明信息 -->
      <div class="flex items-start">
        <div class="w-1.5 h-1.5 bg-gray-400 rounded-full mr-3 mt-2 flex-shrink-0" />
        <div class="flex-1">
          <div class="text-xs opacity-60">
            <p class="mb-2">
              💡 <strong>使用说明：</strong>
            </p>
            <ul class="list-disc list-inside space-y-1">
              <li>自动检测模式会按优先级检测常用代理端口（7890, 7891, 10808, 10809, 1080, 8080）</li>
              <li>代理仅用于更新检查和下载，不影响其他网络请求</li>
              <li>建议启用"仅在中国大陆使用代理"以避免不必要的代理使用</li>
              <li>手动配置的代理优先级低于自动检测</li>
            </ul>
          </div>
        </div>
      </div>
    </n-space>
  </n-spin>
</template>

