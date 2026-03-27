<script setup lang="ts">
</script>

<template>
  <n-card size="small" class="aspect-square" :content-style="{ padding: '8px', display: 'flex', flexDirection: 'column', alignItems: 'center', justifyContent: 'center', gap: '6px' }">
    <n-skeleton circle :width="40" :height="40" />
    <n-skeleton text class="w-2/3" :height="12" />
  </n-card>
</template>
