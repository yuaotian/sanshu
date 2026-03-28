<script setup lang="ts">
interface Props {
  type?: 'popup' | 'main-layout' | 'content'
  lines?: number
  showHeader?: boolean
  showActions?: boolean
}

const _props = withDefaults(defineProps<Props>(), {
  type: 'popup',
  lines: 3,
  showHeader: true,
  showActions: true,
})
</script>

<template>
  <div class="w-full">
    <div v-if="type === 'popup'" class="flex flex-col h-full">
      <div v-if="showHeader" class="flex items-center justify-between p-4 border-b border-gray-200 dark:border-gray-700">
        <n-skeleton circle :width="12" :height="12" />
        <n-skeleton :width="128" :height="16" :sharp="false" />
        <n-skeleton circle :width="24" :height="24" />
      </div>

      <div class="flex-1 p-4 space-y-2">
        <n-skeleton class="w-full mb-4" :height="24" :sharp="false" />
        <n-skeleton
          v-for="i in lines"
          :key="i"
          :class="['mb-2', i === lines ? 'w-3/4' : 'w-full']"
          :height="i === 1 ? 20 : 16"
          :sharp="false"
        />
      </div>

      <div class="p-4 border-t border-gray-200 dark:border-gray-700">
        <n-skeleton class="w-24 mb-2" :height="16" :sharp="false" />
        <n-skeleton class="w-full" :height="80" :sharp="false" />
      </div>

      <div v-if="showActions" class="flex items-center justify-between p-4 border-t border-gray-200 dark:border-gray-700">
        <n-skeleton class="w-16" :height="16" :sharp="false" />
        <div class="flex gap-2">
          <n-skeleton :width="64" :height="32" :sharp="false" />
          <n-skeleton :width="64" :height="32" :sharp="false" />
        </div>
      </div>
    </div>

    <div v-else-if="type === 'main-layout'" class="flex flex-col items-center space-y-6 p-6 pt-12">
      <div class="flex items-center gap-3">
        <n-skeleton circle :width="40" :height="40" />
        <n-skeleton :width="128" :height="32" :sharp="false" />
      </div>

      <div class="flex justify-center">
        <n-skeleton class="w-24" :height="24" :sharp="false" />
      </div>

      <div class="flex justify-center">
        <n-skeleton class="w-48" :height="16" :sharp="false" />
      </div>

      <div class="flex gap-2 justify-center">
        <n-skeleton class="w-16" :height="32" :sharp="false" />
        <n-skeleton class="w-20" :height="32" :sharp="false" />
        <n-skeleton :width="72" :height="32" :sharp="false" />
        <n-skeleton class="w-12" :height="32" :sharp="false" />
      </div>

      <div class="w-full max-w-6xl">
        <div class="grid grid-cols-2 gap-4">
          <div v-for="i in 4" :key="i" class="p-4 rounded-lg border border-gray-200 dark:border-gray-700">
            <n-skeleton circle :width="32" :height="32" class="mb-3" />
            <n-skeleton class="w-full mb-2" :height="20" :sharp="false" />
            <n-skeleton class="w-3/4" :height="16" :sharp="false" />
          </div>
        </div>
      </div>
    </div>

    <div v-else class="space-y-2">
      <n-skeleton
        v-for="i in lines"
        :key="i"
        :class="[
          'mb-3',
          i === 1 ? 'w-full' : 'w-5/6',
          i === lines ? 'w-1/2' : '',
        ]"
        :height="i === 1 ? 24 : 16"
        :sharp="false"
      />
    </div>
  </div>
</template>
