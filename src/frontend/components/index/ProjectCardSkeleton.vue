<script setup lang="ts">
</script>

<template>
  <n-card size="small">
    <n-space vertical :size="8">
      <div class="flex items-start justify-between gap-3">
        <div class="flex-1 min-w-0">
          <div class="flex items-center gap-2 mb-2">
            <n-skeleton width="16px" height="16px" :sharp="false" />
            <n-skeleton width="128px" height="16px" :sharp="false" />
          </div>
          <n-skeleton width="192px" height="12px" :sharp="false" />
        </div>
        <n-skeleton width="64px" height="20px" circle />
      </div>
      <div class="flex items-center gap-3">
        <n-skeleton width="48px" height="16px" :sharp="false" />
        <n-skeleton width="40px" height="16px" :sharp="false" />
        <n-skeleton width="40px" height="16px" :sharp="false" />
      </div>
      <n-skeleton width="96px" height="12px" :sharp="false" />
      <div class="flex items-center gap-2 pt-2 border-t border-border">
        <n-skeleton width="56px" height="20px" :sharp="false" />
        <div class="flex-1" />
        <n-skeleton width="48px" height="24px" :sharp="false" />
        <n-skeleton width="48px" height="24px" :sharp="false" />
      </div>
    </n-space>
  </n-card>
</template>
