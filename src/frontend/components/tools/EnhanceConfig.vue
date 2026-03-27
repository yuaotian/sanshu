<script setup lang="ts">
/**
 * 提示词增强配置面板
 * 复用 acemcp 配置，仅展示 base_url/token 与历史管理
 */
import { invoke } from '@tauri-apps/api/core';
import { useMessage } from 'naive-ui';
import { computed, onMounted, ref, watch } from 'vue';
import ConfigSection from '../common/ConfigSection.vue';

const props = defineProps<{
  active: boolean
  projectRootPath?: string | null
}>()

const message = useMessage()

// 配置状态（复用 acemcp 配置）
const config = ref({
  base_url: '',
  token: '',
  batch_size: 10,
  max_lines_per_blob: 800,
  text_extensions: [] as string[],
  exclude_patterns: [] as string[],
  watch_debounce_ms: 180000,
  // 代理配置
  proxy_enabled: false,
  proxy_host: '127.0.0.1',
  proxy_port: 7890,
  proxy_type: 'http' as 'http' | 'https' | 'socks5',
  proxy_username: '',
  proxy_password: '',
})

const loadingConfig = ref(false)
const historyCount = ref<number | null>(null)
const historyLoading = ref(false)

const hasProject = computed(() => !!props.projectRootPath)

async function loadAcemcpConfig() {
  loadingConfig.value = true
  try {
    const res = await invoke('get_acemcp_config') as any
    config.value = {
      base_url: res.base_url || '',
      token: res.token || '',
      batch_size: res.batch_size,
      max_lines_per_blob: res.max_lines_per_blob,
      text_extensions: res.text_extensions || [],
      exclude_patterns: res.exclude_patterns || [],
      watch_debounce_ms: res.watch_debounce_ms || 180000,
      proxy_enabled: res.proxy_enabled || false,
      proxy_host: res.proxy_host || '127.0.0.1',
      proxy_port: res.proxy_port || 7890,
      proxy_type: res.proxy_type || 'http',
      proxy_username: res.proxy_username || '',
      proxy_password: res.proxy_password || '',
    }
  }
  catch (err) {
    message.error(`加载配置失败: ${err}`)
  }
  finally {
    loadingConfig.value = false
  }
}

async function saveConfig() {
  if (!config.value.base_url || !/^https?:\/\//i.test(config.value.base_url)) {
    message.error('URL无效，需以 http(s):// 开头')
    return
  }

  try {
    await invoke('save_acemcp_config', {
      args: {
        baseUrl: config.value.base_url,
        token: config.value.token,
        batchSize: config.value.batch_size,
        maxLinesPerBlob: config.value.max_lines_per_blob,
        textExtensions: config.value.text_extensions,
        excludePatterns: config.value.exclude_patterns,
        watchDebounceMs: config.value.watch_debounce_ms,
        proxyEnabled: config.value.proxy_enabled,
        proxyHost: config.value.proxy_host,
        proxyPort: config.value.proxy_port,
        proxyType: config.value.proxy_type,
        proxyUsername: config.value.proxy_username,
        proxyPassword: config.value.proxy_password,
      },
    })
    message.success('提示词增强配置已保存')
  }
  catch (err) {
    message.error(`保存失败: ${err}`)
  }
}

async function loadHistoryCount() {
  if (!hasProject.value) {
    historyCount.value = null
    return
  }

  historyLoading.value = true
  try {
    // 中文注释：读取提示词增强历史（非 zhi 交互历史）
    const res = await invoke('get_chat_history', {
      projectRootPath: props.projectRootPath,
      count: 20,
    }) as any[]
    historyCount.value = res.length
  }
  catch (err) {
    // 错误分类处理，提供更友好的提示
    const errMsg = String(err)
    if (errMsg.includes('创建历史管理器失败')) {
      message.error('增强历史管理器初始化失败，请检查项目路径是否正确')
    }
    else if (errMsg.includes('permission') || errMsg.includes('denied')) {
      message.error('读取增强历史文件权限不足，请检查文件访问权限')
    }
    else {
      message.error(`加载增强历史失败: ${errMsg}`)
    }
    historyCount.value = null
  }
  finally {
    historyLoading.value = false
  }
}

async function clearHistory() {
  if (!hasProject.value) {
    message.warning('未检测到项目路径，无法清空历史')
    return
  }

  try {
    // 中文注释：清空提示词增强历史
    await invoke('clear_chat_history', { projectRootPath: props.projectRootPath })
    historyCount.value = 0
    message.success('历史已清空')
  }
  catch (err) {
    // 错误分类处理，提供更友好的提示
    const errMsg = String(err)
    if (errMsg.includes('创建历史管理器失败')) {
      message.error('增强历史管理器初始化失败，请检查项目路径是否正确')
    }
    else if (errMsg.includes('permission') || errMsg.includes('denied')) {
      message.error('写入增强历史文件权限不足，请检查文件访问权限')
    }
    else {
      message.error(`清空增强历史失败: ${errMsg}`)
    }
  }
}

watch(() => props.active, (active) => {
  if (active) {
    loadAcemcpConfig()
    loadHistoryCount()
  }
})

watch(() => props.projectRootPath, () => {
  if (props.active) {
    loadHistoryCount()
  }
})

onMounted(() => {
  if (props.active) {
    loadAcemcpConfig()
    loadHistoryCount()
  }
})
</script>

<template>
  <div class="h-full">
    <n-scrollbar class="max-h-[65vh]">
      <n-space vertical size="medium" class="pr-2 pb-4">
        <!-- 说明提示 -->
        <n-alert type="info" :bordered="false" class="rounded-lg">
          <template #icon>
            <div class="i-carbon-information" />
          </template>
          提示词增强与代码搜索共用 acemcp 配置，修改将同步影响两者。
        </n-alert>

        <!-- 连接设置 -->
        <ConfigSection title="连接设置" description="配置 Augment API 连接信息（复用 acemcp 配置）">
          <div class="mb-4">
            <div class="text-xs text-on-surface-secondary mb-1">
              API 端点 URL
            </div>
            <n-input
              v-model:value="config.base_url"
              size="small"
              :disabled="loadingConfig"
              placeholder="https://d9.api.augmentcode.com"
              clearable
            />
            <div class="text-xs text-on-surface-muted mt-1">
              需包含 http(s):// 前缀
            </div>
          </div>

          <div>
            <div class="text-xs text-on-surface-secondary mb-1">
              认证 Token
            </div>
            <n-input
              v-model:value="config.token"
              size="small"
              :disabled="loadingConfig"
              type="password"
              show-password-on="click"
              placeholder="请输入 token"
              clearable
            />
          </div>

          <div class="flex justify-end mt-3">
            <n-button type="primary" size="small" :loading="loadingConfig" @click="saveConfig">
              <template #icon>
                <div class="i-carbon-save" />
              </template>
              保存配置
            </n-button>
          </div>
        </ConfigSection>

        <!-- 历史管理 -->
        <ConfigSection title="增强历史管理" description="仅保存文本摘要，不包含图片原始数据">
          <div class="flex items-center justify-between">
            <div class="text-sm text-on-surface-secondary">
              <div>当前项目增强历史条数</div>
              <div class="text-xs opacity-70 mt-1">
                <span v-if="historyLoading">加载中...</span>
                <span v-else>{{ historyCount ?? '--' }}</span>
              </div>
            </div>
            <n-button
              type="warning"
              size="small"
              :disabled="!hasProject || historyLoading"
              @click="clearHistory"
            >
              <template #icon>
                <div class="i-carbon-trash-can" />
              </template>
              清空历史
            </n-button>
          </div>

          <n-alert v-if="!hasProject" type="warning" :bordered="false" class="mt-3">
            <template #icon>
              <div class="i-carbon-warning" />
            </template>
            未检测到项目路径，历史统计与清理不可用。
          </n-alert>
        </ConfigSection>
      </n-space>
    </n-scrollbar>
  </div>
</template>

