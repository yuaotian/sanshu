<script setup lang="ts">
/**
 * Context7 文档查询工具配置组件
 * 包含：API Key 配置、连接测试
 */
import { invoke } from '@tauri-apps/api/core';
import { useMessage } from 'naive-ui';
import { onMounted, ref } from 'vue';
import ConfigSection from '../common/ConfigSection.vue';

const props = defineProps<{ active: boolean }>()
const message = useMessage()

// 配置状态
const config = ref({ api_key: '' })

// 测试状态
const testLoading = ref(false)
const testResult = ref<{ success: boolean, message: string, preview?: string } | null>(null)
const testLibrary = ref('spring-projects/spring-framework')
const testTopic = ref('core')

// 常用库数据
const popularLibs = [
  { label: 'Spring Framework', value: 'spring-projects/spring-framework', category: 'Java' },
  { label: 'Spring Boot', value: 'spring-projects/spring-boot', category: 'Java' },
  { label: 'MyBatis', value: 'mybatis/mybatis-3', category: 'Java' },
  { label: 'React', value: 'facebook/react', category: '前端' },
  { label: 'Vue.js', value: 'vuejs/vue', category: '前端' },
  { label: 'Next.js', value: 'vercel/next.js', category: '前端' },
  { label: 'FastAPI', value: 'tiangolo/fastapi', category: '后端' },
  { label: 'Tokio', value: 'tokio-rs/tokio', category: 'Rust' },
  { label: 'Tauri', value: 'tauri-apps/tauri', category: 'Rust' },
]

// --- 操作函数 ---

async function loadConfig() {
  try {
    const res = await invoke('get_context7_config') as { api_key?: string }
    config.value = { api_key: res.api_key || '' }
  }
  catch (err) {
    message.error(`加载配置失败: ${err}`)
  }
}

async function saveConfig() {
  try {
    await invoke('save_context7_config', { apiKey: config.value.api_key })
    message.success('Context7 配置已保存')
  }
  catch (err) {
    message.error(`保存失败: ${err}`)
  }
}

async function runTest() {
  testLoading.value = true
  testResult.value = null
  try {
    const res = await invoke('test_context7_connection', {
      library: testLibrary.value || null,
      topic: testTopic.value || null,
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
  <div class="h-full">
    <n-scrollbar class="max-h-[65vh]">
      <n-space vertical size="medium" class="pr-2 pb-4">
        <!-- 介绍提示 -->
        <n-alert type="info" :bordered="false" class="rounded-lg">
          <template #icon>
            <div class="i-carbon-information" />
          </template>
          Context7 提供最新的框架和库文档查询服务。
        </n-alert>

        <!-- 认证设置 -->
        <ConfigSection title="认证设置" description="配置 Context7 API Key 以获得更高的速率限制">
          <div class="flex flex-col gap-2">
            <div>
              <div class="text-xs text-on-surface-secondary mb-1">API Key (可选)</div>
              <n-input
                v-model:value="config.api_key"
                type="password"
                show-password-on="click"
                size="small"
                placeholder="留空即使用免费模式"
                clearable
              />
              <div class="text-[11px] text-on-surface-muted mt-1">
                免费模式有限制。获取 Key:
                <a href="https://context7.com/dashboard" target="_blank" class="text-primary hover:underline no-underline">context7.com</a>
              </div>
            </div>
            <div class="flex justify-end">
              <n-button type="primary" size="small" @click="saveConfig">
                <template #icon>
                  <div class="i-carbon-save" />
                </template>
                保存配置
              </n-button>
            </div>
          </div>
        </ConfigSection>

        <!-- 连接测试 -->
        <ConfigSection title="连接与查询测试" description="测试是否能成功解析指定库的文档">
          <div class="flex flex-col gap-2">
            <div>
              <div class="text-xs text-on-surface-secondary mb-1">测试目标库</div>
              <n-auto-complete
                v-model:value="testLibrary"
                :options="popularLibs.map(l => ({ label: l.label, value: l.value }))"
                placeholder="owner/repo (e.g. facebook/react)"
                size="small"
                clearable
              />
            </div>
            <div>
              <div class="text-xs text-on-surface-secondary mb-1">查询主题 (可选)</div>
              <n-input v-model:value="testTopic" size="small" placeholder="e.g. routing, state management" />
            </div>
            <div class="flex justify-end">
              <n-button
                secondary
                type="info"
                size="small"
                :loading="testLoading"
                :disabled="!testLibrary"
                @click="runTest"
              >
                <template #icon>
                  <div class="i-carbon-play" />
                </template>
                测试查询
              </n-button>
            </div>
            <n-alert
              v-if="testResult"
              :type="testResult.success ? 'success' : 'error'"
              :title="testResult.success ? '测试成功' : '测试失败'"
              :bordered="false"
            >
              {{ testResult.message }}
              <div v-if="testResult.preview" class="mt-2">
                <div class="text-xs text-on-surface-muted mb-1">响应预览:</div>
                <n-scrollbar style="max-height: 120px">
                  <pre class="m-0 p-2 text-[11px] font-mono whitespace-pre-wrap break-all leading-normal rounded bg-container border border-border">{{ testResult.preview }}</pre>
                </n-scrollbar>
              </div>
            </n-alert>
          </div>
        </ConfigSection>

        <!-- 常用库参考 -->
        <div class="pb-2">
          <div class="text-xs font-medium text-on-surface-secondary mb-2">
            常用库参考
          </div>
          <n-space size="small">
            <n-tag
              v-for="lib in popularLibs"
              :key="lib.value"
              size="small"
              class="cursor-pointer transition-all duration-200"
              :bordered="false"
              @click="testLibrary = lib.value"
            >
              {{ lib.label }}
            </n-tag>
          </n-space>
        </div>
      </n-space>
    </n-scrollbar>
  </div>
</template>

