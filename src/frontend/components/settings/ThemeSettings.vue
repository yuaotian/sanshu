<script setup>
import { primaryColors, themeColors } from '../../theme/colors'

defineProps({
  currentTheme: {
    type: String,
    required: true,
  },
})

defineEmits(['themeChange'])
</script>

<template>
  <!-- 设置内容 -->
  <div class="flex items-center justify-between">
    <div class="flex items-center">
      <div class="w-1.5 h-1.5 bg-primary-500 rounded-full mr-3 flex-shrink-0" />
      <div>
        <div class="text-sm font-medium leading-relaxed">
          界面主题
        </div>
        <div class="text-xs opacity-60">
          选择浅色或深色主题
        </div>
      </div>
    </div>
    <n-space>
      <!-- 浅色主题 -->
      <n-button
        :type="currentTheme === 'light' ? 'primary' : 'default'"
        size="small"
        @click="$emit('themeChange', 'light')"
      >
        <template #icon>
          <div
            class="w-3 h-3 rounded-full border transition-all duration-200"
            :style="{
              backgroundColor: themeColors.light.surface,
              borderColor:
                currentTheme === 'light'
                  ? primaryColors[500]
                  : themeColors.light.surface300,
            }"
          />
        </template>
        浅色
      </n-button>

      <!-- 深色主题 -->
      <n-button
        :type="currentTheme === 'dark' ? 'primary' : 'default'"
        size="small"
        @click="$emit('themeChange', 'dark')"
      >
        <template #icon>
          <div
            class="w-3 h-3 rounded-full border transition-all duration-200"
            :style="{
              backgroundColor: themeColors.light.surface800,
              borderColor:
                currentTheme === 'dark'
                  ? primaryColors[500]
                  : themeColors.light.surface300,
            }"
          />
        </template>
        深色
      </n-button>
    </n-space>
  </div>
</template>
