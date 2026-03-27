<script setup lang="ts">
import { invoke } from '@tauri-apps/api/core'
import { NSpace, NTag, useMessage } from 'naive-ui'
import { onMounted, ref } from 'vue'

const message = useMessage()
const currentVersion = ref('')

async function openGitHub() {
  try {
    await invoke('open_external_url', { url: 'https://github.com/yueby/sanshu' })
    message.success('正在打开GitHub页面...')
  }
  catch (error) {
    message.error(error instanceof Error ? error.message : '打开GitHub失败')
  }
}

async function openGitHubStars() {
  try {
    await invoke('open_external_url', { url: 'https://github.com/yueby/sanshu/stargazers' })
    message.success('正在打开Star页面...')
  }
  catch (error) {
    message.error(error instanceof Error ? error.message : '打开Star页面失败')
  }
}

// 功能亮点配置 (高定美学配色)
const features = [
  {
    label: '智能交互',
    icon: 'i-carbon-chat',
    colorClass: '!bg-blue-50 !text-blue-600 !border-blue-200 dark:!bg-blue-900/30 dark:!text-blue-300 dark:!border-blue-700/50',
  },
  {
    label: '全局记忆',
    icon: 'i-carbon-data-base',
    colorClass: '!bg-violet-50 !text-violet-600 !border-violet-200 dark:!bg-violet-900/30 dark:!text-violet-300 dark:!border-violet-700/50',
  },
  {
    label: '语义搜索',
    icon: 'i-carbon-search',
    colorClass: '!bg-emerald-50 !text-emerald-600 !border-emerald-200 dark:!bg-emerald-900/30 dark:!text-emerald-300 dark:!border-emerald-700/50',
  },
  {
    label: '框架文档',
    icon: 'i-carbon-document',
    colorClass: '!bg-orange-50 !text-orange-600 !border-orange-200 dark:!bg-orange-900/30 dark:!text-orange-300 dark:!border-orange-700/50',
  },
  {
    label: 'UI/UX 设计',
    icon: 'i-carbon-paint-brush',
    colorClass: '!bg-pink-50 !text-pink-600 !border-pink-200 dark:!bg-pink-900/30 dark:!text-pink-300 dark:!border-pink-700/50',
  },
  {
    label: '图标工坊',
    icon: 'i-carbon-image',
    colorClass: '!bg-indigo-50 !text-indigo-600 !border-indigo-200 dark:!bg-indigo-900/30 dark:!text-indigo-300 dark:!border-indigo-700/50',
  },
]

onMounted(async () => {
  try {
    const version = await invoke('get_current_version') as string
    if (version) currentVersion.value = version
  }
  catch { /* use default */ }
})
</script>

<template>
  <n-card
    size="small"
    class="transition-all duration-200 hover:shadow-md"
  >
    <!-- 主要内容区域 -->
    <div class="flex items-center justify-between mb-2">
      <!-- 左侧：项目信息 -->
      <div class="flex items-center gap-3">
        <div class="w-8 h-8 rounded-lg bg-blue-100 dark:bg-blue-900 flex items-center justify-center">
          <div class="i-carbon-logo-github text-blue-600 dark:text-blue-400" />
        </div>
        <div>
          <h3 class="font-semibold text-gray-900 dark:text-white text-sm">
            三术 {{ currentVersion ? `v${currentVersion}` : '' }}
          </h3>
          <p class="text-xs text-gray-500 dark:text-gray-400">
            智能代码审查工具，支持MCP协议集成
          </p>
        </div>
      </div>

    </div>

    <!-- 功能亮点标签云 -->
    <div class="py-3 border-b border-gray-100 dark:border-gray-700">
      <n-space size="small" :wrap="true">
        <n-tag
          v-for="feature in features"
          :key="feature.label"
          size="small"
          :bordered="true"
          round
          class="transition-colors duration-300"
          :class="feature.colorClass"
        >
          <template #icon>
            <div :class="[feature.icon, 'text-xs']" />
          </template>
          {{ feature.label }}
        </n-tag>
      </n-space>
    </div>

    <!-- 底部：GitHub区域 -->
    <div class="flex items-center justify-between border-t border-gray-100 dark:border-gray-700 pt-2">
      <div class="flex items-center gap-1">
        <n-button
          size="small"
          type="primary"
          @click="openGitHub"
        >
          <template #icon>
            <div class="i-carbon-logo-github" />
          </template>
          GitHub
        </n-button>

        <n-button
          size="small"
          secondary
          @click="openGitHubStars"
        >
          <template #icon>
            <div class="i-carbon-star text-yellow-500" />
          </template>
          Star
        </n-button>
      </div>

      <!-- 弱化的提示文字 -->
      <p class="text-xs text-gray-400 dark:text-gray-500">
        如果对您有帮助，请给我们一个 Star ⭐
      </p>
    </div>
  </n-card>
</template>
