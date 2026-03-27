<script setup lang="ts">
defineProps<{
  title?: string
  description?: string
  noCard?: boolean
}>()
</script>

<template>
  <div class="mb-0.5">
    <div v-if="title || description" class="mb-2">
      <div v-if="title" class="text-sm font-semibold text-on-surface flex items-center gap-2">
        <slot name="icon" />
        {{ title }}
      </div>
      <n-text v-if="description" depth="3" class="text-xs mt-1 block">
        {{ description }}
      </n-text>
    </div>
    <n-card v-if="!noCard" size="small">
      <slot />
    </n-card>
    <div v-else>
      <slot />
    </div>
  </div>
</template>
