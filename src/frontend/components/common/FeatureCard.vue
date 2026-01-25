<script setup lang="ts">
import { NSkeleton, NSpace, NCard } from 'naive-ui'

export interface Feature {
  icon: string
  title: string
  titleClass?: string
  subtitle: string
  iconWrapperClass: string
  features: string[]
}

defineProps<{
  feature?: Feature
  loading?: boolean
}>()
</script>

<template>
  <n-card size="small" class="h-full transition-all duration-300 hover:shadow-md hover:-translate-y-0.5">
    <!-- 统一的 Header 插槽 -->
    <template #header>
      <!-- 加载状态头部 -->
      <n-space v-if="loading" align="center">
        <n-skeleton height="40px" width="40px" :sharp="false" />
        <div>
          <n-skeleton text width="120px" class="mb-1" />
          <n-skeleton text width="80px" />
        </div>
      </n-space>

      <!-- 真实内容头部 -->
      <n-space v-else-if="feature" align="center">
        <!-- 图标容器 -->
        <div
          class="w-10 h-10 rounded-xl flex items-center justify-center transition-colors duration-300"
          :class="feature.iconWrapperClass"
        >
          <!-- 图标 -->
          <div :class="feature.icon" />
        </div>

        <!-- 标题信息 -->
        <div>
          <div
            class="text-base font-semibold leading-tight"
            :class="feature.titleClass || 'text-gray-900 dark:text-gray-100'"
          >
            {{ feature.title }}
          </div>
          <div class="text-xs text-gray-500 dark:text-gray-400 mt-0.5">
            {{ feature.subtitle }}
          </div>
        </div>
      </n-space>
    </template>

    <!-- 加载状态内容 -->
    <n-space v-if="loading" vertical size="small" class="mt-2">
      <n-skeleton text :repeat="4" />
    </n-space>

    <!-- 真实内容列表 -->
    <div v-else-if="feature" class="grid gap-2">
      <div
        v-for="(item, index) in feature.features"
        :key="index"
        class="flex items-start text-xs group"
      >
        <div class="mt-1.5 w-1 h-1 rounded-full bg-current opacity-40 mr-2 shrink-0 group-hover:opacity-80 transition-opacity" />
        <span class="text-gray-600 dark:text-gray-400 group-hover:text-gray-900 dark:group-hover:text-gray-200 transition-colors leading-relaxed">
          {{ item }}
        </span>
      </div>
    </div>
  </n-card>
</template>
