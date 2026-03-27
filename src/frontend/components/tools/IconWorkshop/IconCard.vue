<script setup lang="ts">
import { computed, ref } from 'vue'
import type { IconItem } from '../../../types/icon'
import { sanitizeSvg } from '../../../utils/sanitize'

interface Props {
  icon: IconItem
  selected?: boolean
}
const props = withDefaults(defineProps<Props>(), {
  selected: false,
})

const emit = defineEmits<{
  toggle: []
  copy: []
  dblclick: []
  contextmenu: [event: MouseEvent]
}>()

const isHovered = ref(false)

const displayName = computed(() => {
  const name = props.icon.name
  return name.length > 12 ? `${name.slice(0, 10)}...` : name
})

const svgContent = computed(() => {
  if (!props.icon.svgContent)
    return null
  return sanitizeSvg(props.icon.svgContent
    .replace(/\s*style="[^"]*"/g, '')
    .replace(/\s*width="[^"]*"/g, ' width="100%"')
    .replace(/\s*height="[^"]*"/g, ' height="100%"'))
})

function handleClick() {
  emit('toggle')
}

function handleDblClick() {
  emit('dblclick')
}

function handleContextMenu(e: MouseEvent) {
  e.preventDefault()
  emit('contextmenu', e)
}

function handleCopy(e: Event) {
  e.stopPropagation()
  emit('copy')
}
</script>

<template>
  <n-card
    size="small"
    hoverable
    class="icon-card cursor-pointer"
    :class="{ 'icon-card--selected': selected }"
    :content-style="{ padding: '8px', display: 'flex', flexDirection: 'column', alignItems: 'center', justifyContent: 'center', gap: '6px' }"
    @click="handleClick"
    @dblclick="handleDblClick"
    @contextmenu="handleContextMenu"
    @mouseenter="isHovered = true"
    @mouseleave="isHovered = false"
  >
    <!-- 选中标记 -->
    <div v-if="selected" class="selected-badge">
      <div class="i-carbon-checkmark text-white text-xs" />
    </div>

    <!-- 图标预览 -->
    <div class="icon-preview">
      <div
        v-if="svgContent"
        class="svg-container"
        v-html="svgContent"
      />
      <div
        v-else-if="icon.fontClass"
        class="font-icon"
        :class="icon.fontClass"
      />
      <div v-else class="icon-placeholder">
        <div class="i-carbon-image text-2xl opacity-30" />
      </div>
    </div>

    <!-- 图标名称 -->
    <div class="icon-name" :title="icon.name">
      {{ displayName }}
    </div>

    <!-- 悬停操作 -->
    <div v-if="isHovered" class="icon-actions">
      <n-tooltip>
        <template #trigger>
          <n-button
            size="tiny"
            circle
            quaternary
            @click="handleCopy"
          >
            <template #icon>
              <div class="i-carbon-copy" />
            </template>
          </n-button>
        </template>
        复制 SVG
      </n-tooltip>
    </div>
  </n-card>
</template>

<style scoped>
.icon-card {
  position: relative;
  aspect-ratio: 1;
}

.icon-card--selected {
  outline: 2px solid var(--color-primary-500);
  outline-offset: -2px;
}

.selected-badge {
  position: absolute;
  top: 4px;
  right: 4px;
  width: 18px;
  height: 18px;
  border-radius: 50%;
  background: var(--color-primary-500);
  display: flex;
  align-items: center;
  justify-content: center;
  z-index: 1;
}

.icon-preview {
  width: 40px;
  height: 40px;
  display: flex;
  align-items: center;
  justify-content: center;
  color: var(--color-on-surface);
  overflow: hidden;
  flex-shrink: 0;
}

.svg-container {
  width: 100%;
  height: 100%;
  display: flex;
  align-items: center;
  justify-content: center;
  overflow: hidden;
}

.svg-container :deep(svg) {
  width: 100%;
  height: 100%;
  max-width: 40px;
  max-height: 40px;
  object-fit: contain;
}

.font-icon {
  font-size: 32px;
}

.icon-placeholder {
  width: 100%;
  height: 100%;
  display: flex;
  align-items: center;
  justify-content: center;
}

.icon-name {
  font-size: 11px;
  color: var(--color-on-surface-secondary);
  text-align: center;
  max-width: 100%;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.icon-actions {
  position: absolute;
  bottom: 4px;
  right: 4px;
  z-index: 1;
}
</style>
