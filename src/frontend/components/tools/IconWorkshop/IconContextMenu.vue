<script setup lang="ts">
/**
 * 图标右键上下文菜单
 * 提供"打开编辑器 / 复制 SVG"快捷操作
 * 从 IconPopupMode.vue（原上帝组件）拆分而来
 */

interface Props {
  /** 是否可见 */
  visible: boolean
  /** 菜单位置（相对视口） */
  position: { x: number, y: number }
}

defineProps<Props>()

const emit = defineEmits<{
  /** 打开 SVG 编辑器 */
  openEditor: []
  /** 复制 SVG 内容 */
  copySvg: []
  /** 关闭菜单 */
  close: []
}>()
</script>

<template>
  <Teleport to="body">
    <transition
      enter-active-class="transition duration-100 ease-out"
      enter-from-class="opacity-0 scale-95"
      enter-to-class="opacity-100 scale-100"
      leave-active-class="transition duration-75 ease-in"
      leave-from-class="opacity-100 scale-100"
      leave-to-class="opacity-0 scale-95"
    >
      <div
        v-if="visible"
        class="fixed z-50 min-w-40 rounded-lg border border-border bg-surface-variant shadow-xl py-1"
        :style="{ left: `${position.x}px`, top: `${position.y}px` }"
        @click.stop
      >
        <div
          class="px-3 py-2 text-sm cursor-pointer hover:bg-surface-100 flex items-center gap-2"
          @click="emit('openEditor')"
        >
          <div class="i-carbon-color-palette text-base" />
          <span>打开 SVG 编辑器</span>
        </div>
        <div
          class="px-3 py-2 text-sm cursor-pointer hover:bg-surface-100 flex items-center gap-2"
          @click="emit('copySvg')"
        >
          <div class="i-carbon-copy text-base" />
          <span>复制 SVG</span>
        </div>
      </div>
    </transition>
    <!-- 点击遮罩关闭菜单 -->
    <div
      v-if="visible"
      class="fixed inset-0 z-40"
      @click="emit('close')"
      @contextmenu.prevent="emit('close')"
    />
  </Teleport>
</template>
