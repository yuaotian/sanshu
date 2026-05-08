<script setup lang="ts">
/**
 * Tavily AI 搜索工具配置组件
 * 包含：API Key 配置（必填）、连接测试
 */
import { invoke } from '@tauri-apps/api/core'
import { useMessage } from 'naive-ui'
import { onMounted, ref } from 'vue'
import ConfigSection from '../common/ConfigSection.vue'

const props = defineProps<{ active: boolean }>()
const message = useMessage()

// 配置状态
const config = ref({ api_key: '' })

// 测试状态
const testLoading = ref(false)
const testResult = ref<{ success: boolean, message: string, preview?: string } | null>(null)

// --- 操作函数 ---

async function loadConfig() {
  try {
    const res = await invoke('get_tavily_config') as { api_key?: string }
    config.value = { api_key: res.api_key || '' }
  }
  catch (err) {
    message.error(`加载配置失败: ${err}`)
  }
}

async function saveConfig() {
  if (!config.value.api_key.trim()) {
    message.warning('API Key 不能为空')
    return
  }
  try {
    await invoke('save_tavily_config', { apiKey: config.value.api_key })
    message.success('Tavily 配置已保存')
  }
  catch (err) {
    message.error(`保存失败: ${err}`)
  }
}

async function runTest() {
  const key = config.value.api_key.trim()
  if (!key) {
    message.warning('请先填写 API Key')
    return
  }
  testLoading.value = true
  testResult.value = null
  try {
    const res = await invoke('test_tavily_connection', {
      apiKey: key,
    }) as any

    testResult.value = res
    if (res.success)
      message.success('测试成功')
    else message.error(res.message)
  }
  catch (err) {
    testResult.value = { success: false, message: `System Error: ${err}` }
    message.error(`测试异常: ${err}`)
  }
  finally {
    testLoading.value = false
  }
}

// 组件挂载
onMounted(() => {
  if (props.active)
    loadConfig()
})

defineExpose({ saveConfig })
</script>

<template>
  <div class="tavily-config">
    <n-scrollbar class="config-scrollbar">
      <n-space vertical size="large" class="config-content">
        <!-- 介绍提示 -->
        <n-alert type="info" :bordered="false" class="intro-alert">
          <template #icon>
            <div class="i-carbon-information" />
          </template>
          Tavily 提供专为 AI 优化的搜索引擎 API。免费计划每月 1000 信用点，无需信用卡。
        </n-alert>

        <!-- 认证设置 -->
        <ConfigSection title="认证设置" description="配置 Tavily API Key（必填）">
          <n-form-item label="API Key" required>
            <n-input
              v-model:value="config.api_key"
              type="password"
              show-password-on="click"
              placeholder="输入 Tavily API Key (tvly-...)"
              clearable
            />
            <template #feedback>
              <span class="form-feedback">
                前往
                <a href="https://app.tavily.com/home" target="_blank" class="link">app.tavily.com</a>
                注册并获取免费 API Key
              </span>
            </template>
          </n-form-item>

          <div class="flex justify-end gap-2 mt-3">
            <n-button type="primary" @click="saveConfig">
              <template #icon>
                <div class="i-carbon-save" />
              </template>
              保存配置
            </n-button>
          </div>
        </ConfigSection>

        <!-- 连接测试 -->
        <ConfigSection title="连接测试" description="验证 API Key 是否有效">
          <n-space vertical size="medium">
            <div class="flex justify-end">
              <n-button
                secondary
                type="info"
                :loading="testLoading"
                :disabled="!config.api_key.trim()"
                @click="runTest"
              >
                <template #icon>
                  <div class="i-carbon-play" />
                </template>
                测试连接
              </n-button>
            </div>

            <!-- 测试结果 -->
            <transition name="fade">
              <div v-if="testResult" class="test-result" :class="testResult.success ? 'success' : 'error'">
                <div class="result-header">
                  <div :class="testResult.success ? 'i-carbon-checkmark-filled' : 'i-carbon-warning-filled'" />
                  {{ testResult.success ? '测试成功' : '测试失败' }}
                </div>
                <div class="result-message">
                  {{ testResult.message }}
                </div>

                <div v-if="testResult.preview" class="result-preview">
                  <div class="preview-label">
                    响应预览:
                  </div>
                  <div class="preview-content">
                    {{ testResult.preview }}
                  </div>
                </div>
              </div>
            </transition>
          </n-space>
        </ConfigSection>

        <!-- 费用说明 -->
        <div class="quick-libs">
          <div class="libs-label">
            信用点说明
          </div>
          <n-space size="small" vertical>
            <div class="text-xs text-on-surface-secondary">
              <n-tag size="small" type="success" :bordered="false">Basic Search</n-tag> 1 信用点/次
              <n-tag size="small" type="warning" :bordered="false" class="ml-2">Advanced Search</n-tag> 2 信用点/次
            </div>
            <div class="text-xs text-on-surface-secondary">
              <n-tag size="small" type="info" :bordered="false">Extract</n-tag> 每 5 次成功提取消耗 1 信用点
            </div>
            <div class="text-xs opacity-50">
              免费计划：每月 1000 信用点 | 每分钟 100 次请求
            </div>
          </n-space>
        </div>
      </n-space>
    </n-scrollbar>
  </div>
</template>

<style scoped>
.tavily-config {
  height: 100%;
}

.config-scrollbar {
  max-height: 65vh;
}

.config-content {
  padding-right: 8px;
  padding-bottom: 16px;
}

/* 介绍提示 */
.intro-alert {
  border-radius: 8px;
}

/* 表单反馈 */
.form-feedback {
  font-size: 11px;
  color: var(--color-on-surface-muted, #9ca3af);
}

.link {
  color: #f97316;
  text-decoration: none;
}

.link:hover {
  text-decoration: underline;
}

/* 测试结果 */
.test-result {
  padding: 12px;
  border-radius: 8px;
  border: 1px solid;
}

.test-result.success {
  background: rgba(34, 197, 94, 0.08);
  border-color: rgba(34, 197, 94, 0.3);
}

.test-result.error {
  background: rgba(239, 68, 68, 0.08);
  border-color: rgba(239, 68, 68, 0.3);
}

:root.dark .test-result.success {
  background: rgba(34, 197, 94, 0.15);
  border-color: rgba(34, 197, 94, 0.4);
}

:root.dark .test-result.error {
  background: rgba(239, 68, 68, 0.15);
  border-color: rgba(239, 68, 68, 0.4);
}

.result-header {
  display: flex;
  align-items: center;
  gap: 6px;
  font-size: 13px;
  font-weight: 500;
  margin-bottom: 4px;
}

.test-result.success .result-header {
  color: #22c55e;
}

.test-result.error .result-header {
  color: #ef4444;
}

.result-message {
  font-size: 12px;
  color: var(--color-on-surface-secondary, #6b7280);
}

:root.dark .result-message {
  color: #9ca3af;
}

.result-preview {
  margin-top: 12px;
}

.preview-label {
  font-size: 11px;
  color: var(--color-on-surface-muted, #9ca3af);
  margin-bottom: 6px;
}

.preview-content {
  padding: 10px;
  border-radius: 6px;
  font-size: 11px;
  font-family: ui-monospace, monospace;
  max-height: 150px;
  overflow-y: auto;
  background: var(--color-container, rgba(128, 128, 128, 0.08));
  border: 1px solid var(--color-border, rgba(128, 128, 128, 0.2));
}

:root.dark .preview-content {
  background: rgba(24, 24, 28, 0.8);
  border-color: rgba(255, 255, 255, 0.08);
}

/* 信用点说明 */
.quick-libs {
  padding-bottom: 8px;
}

.libs-label {
  font-size: 12px;
  font-weight: 500;
  color: var(--color-on-surface-secondary, #6b7280);
  margin-bottom: 8px;
}

:root.dark .libs-label {
  color: #9ca3af;
}

/* 过渡动画 */
.fade-enter-active, .fade-leave-active {
  transition: opacity 0.3s ease;
}

.fade-enter-from, .fade-leave-to {
  opacity: 0;
}
</style>
