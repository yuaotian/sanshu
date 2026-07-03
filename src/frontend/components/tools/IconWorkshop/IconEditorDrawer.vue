<script setup lang="ts">
/**
 * 图标编辑抽屉
 * 固定在弹窗右侧的 SVG 编辑面板（替代原可拖拽浮动编辑器窗），
 * 包含实时预览、外观/变换/线条/元素级编辑与保存操作
 * 从 IconPopupMode.vue（原上帝组件）拆分而来
 */
import { invoke } from '@tauri-apps/api/core'
import { useMessage } from 'naive-ui'
import { computed, ref, watch } from 'vue'
import type { IconEditor } from '../../../composables/useIconEditor'
import type { IconFormat, IconItem } from '../../../types/icon'

interface Props {
  /** 编辑器实例（由父组件创建，保证与右键菜单等共享状态） */
  editor: IconEditor
  /** 当前选中的图标列表 */
  selectedIcons: IconItem[]
  /** 初始保存路径 */
  initialSavePath?: string
  /** 是否禁用交互（保存进行中） */
  disabled?: boolean
}

const props = withDefaults(defineProps<Props>(), {
  initialSavePath: '',
  disabled: false,
})

const emit = defineEmits<{
  /** 保存当前编辑图标 */
  save: [payload: { savePath: string, format: IconFormat, pngSize?: number }]
  /** 收起抽屉 */
  close: []
}>()

const message = useMessage()

// 从编辑器实例解构响应式状态与方法（保持 ref 响应性）
const {
  activeIconId,
  activeIcon,
  activeState,
  editorStatus,
  editorPreviewSvg,
  previewUpdating,
  mergedSwatches,
  elementSearch,
  activeElementOptions,
  filteredElementOptions,
  activeElementKey,
  activeElementStyle,
  updateActiveState,
  toggleActiveState,
  applySizePreset,
  updateActiveElementStyle,
  resetActiveElementStyle,
  resetActiveEditor,
  getEditedSvg,
} = props.editor

// ============ 本地 UI 状态 ============

const sizePresets = [16, 24, 32, 48, 64, 96, 128]
const pngSizePresets = [16, 32, 64, 128, 256]
const previewScaleOptions = [
  { label: '50%', value: 50 },
  { label: '75%', value: 75 },
  { label: '100%', value: 100 },
  { label: '125%', value: 125 },
  { label: '150%', value: 150 },
  { label: '200%', value: 200 },
]

const previewBackground = ref<'grid' | 'light' | 'dark'>('grid')
const previewScale = ref(100)
const saveFormat = ref<IconFormat>('svg')
const pngSize = ref(128)
const savePath = ref(props.initialSavePath || 'assets/icons')
const collapseExpanded = ref(['appearance', 'transform'])

watch(() => props.initialSavePath, (value) => {
  if (value)
    savePath.value = value
})

// ============ 计算属性 ============

const previewBusy = computed(() => editorStatus.value === 'loading' || previewUpdating.value)
const needsPngSize = computed(() => saveFormat.value === 'png' || saveFormat.value === 'both')

const previewBackgroundClass = computed(() => {
  if (previewBackground.value === 'light')
    return 'bg-stone-50'
  return 'bg-slate-900'
})

const previewScaleClass = computed(() => {
  const map: Record<number, string> = {
    50: 'preview-scale-50',
    75: 'preview-scale-75',
    100: 'preview-scale-100',
    125: 'preview-scale-125',
    150: 'preview-scale-150',
    200: 'preview-scale-200',
  }
  return map[previewScale.value] || 'preview-scale-100'
})

const editorStatusLabel = computed(() => {
  if (editorStatus.value === 'loading')
    return '加载中'
  if (editorStatus.value === 'ready')
    return '就绪'
  if (editorStatus.value === 'error')
    return '加载失败'
  return '未选择'
})

const editorStatusTagType = computed(() => {
  if (editorStatus.value === 'loading')
    return 'warning'
  if (editorStatus.value === 'ready')
    return 'success'
  if (editorStatus.value === 'error')
    return 'error'
  return 'default'
})

// ============ 操作方法 ============

function updatePngSize(value: number | null) {
  if (value === null)
    return
  pngSize.value = value
}

// 复制编辑后的 SVG 到剪贴板
async function copyEditedSvg() {
  const icon = activeIcon.value
  const edited = icon ? getEditedSvg(icon) : editorPreviewSvg.value
  if (!edited) {
    message.warning('暂无可复制的 SVG')
    return
  }
  try {
    await navigator.clipboard.writeText(edited)
    message.success('已复制编辑后的 SVG')
  }
  catch (error) {
    console.error('复制 SVG 失败:', error)
    message.error('复制失败，请稍后重试')
  }
}

// 打开系统目录选择对话框
async function selectDirectory() {
  try {
    const result = await invoke<string | null>('select_icon_save_directory', {
      defaultPath: savePath.value,
    })
    if (result)
      savePath.value = result
  }
  catch (error) {
    console.error('选择目录失败:', error)
    message.error('选择目录失败')
  }
}

// 保存当前编辑的图标
function handleSave() {
  if (!activeIcon.value) {
    message.warning('请先选择要编辑的图标')
    return
  }
  if (!savePath.value.trim()) {
    message.warning('请填写保存路径')
    return
  }
  emit('save', {
    savePath: savePath.value,
    format: saveFormat.value,
    pngSize: needsPngSize.value ? pngSize.value : undefined,
  })
}
</script>

<template>
  <div class="h-full flex flex-col rounded-2xl border border-slate-200/70 dark:border-white/10 bg-white/90 dark:bg-[#1f1f23] overflow-hidden">
    <!-- 标题栏 -->
    <div class="flex-shrink-0 flex items-center justify-between px-4 py-3 border-b border-slate-200/70 dark:border-white/5 bg-slate-50/80 dark:bg-[#252529]">
      <div class="flex items-center gap-2 select-none">
        <div class="i-carbon-color-palette text-lg text-slate-500" />
        <span class="font-semibold text-slate-700 dark:text-gray-200">SVG 编辑器</span>
        <n-tag :type="editorStatusTagType" size="small" round>
          {{ editorStatusLabel }}
        </n-tag>
      </div>
      <n-button size="small" quaternary circle @click="emit('close')">
        <template #icon>
          <div class="i-carbon-side-panel-close" />
        </template>
      </n-button>
    </div>

    <!-- 主内容区（纵向滚动） -->
    <n-scrollbar class="flex-1 min-h-0">
      <div class="p-4 space-y-4">
        <!-- 当前图标选择 -->
        <div class="rounded-xl border border-slate-200/70 dark:border-white/10 bg-white/80 dark:bg-[#1a1a1d] p-3 space-y-2">
          <div class="flex items-center justify-between">
            <label class="text-xs font-semibold text-slate-400 dark:text-gray-500 uppercase tracking-wider">当前图标</label>
            <span class="text-xs text-slate-400">{{ selectedIcons.length }} 个已选</span>
          </div>
          <n-select
            v-model:value="activeIconId"
            size="small"
            :options="selectedIcons.map(icon => ({ label: icon.name, value: icon.id }))"
            placeholder="请选择图标"
            :disabled="!selectedIcons.length"
            virtual-scroll
          />
        </div>

        <!-- 实时预览 -->
        <div class="rounded-2xl border border-slate-200/70 dark:border-white/10 bg-slate-50/70 dark:bg-[#141417] p-4 flex flex-col gap-3">
          <div class="flex flex-wrap items-center justify-between gap-2">
            <label class="text-xs font-semibold text-slate-400 dark:text-gray-500 uppercase tracking-wider">实时预览</label>
            <div class="flex flex-wrap items-center gap-2">
              <n-radio-group v-model:value="previewBackground" size="small">
                <n-radio-button value="grid">
                  网格
                </n-radio-button>
                <n-radio-button value="light">
                  浅色
                </n-radio-button>
                <n-radio-button value="dark">
                  深色
                </n-radio-button>
              </n-radio-group>
              <n-select
                v-model:value="previewScale"
                size="small"
                :options="previewScaleOptions"
                class="w-24"
              />
            </div>
          </div>

          <div class="relative h-56 rounded-xl border border-slate-200/60 dark:border-white/10 overflow-hidden" :class="previewBackgroundClass">
            <div v-if="previewBackground === 'grid'" class="absolute inset-0 pattern-grid opacity-15 pointer-events-none" />
            <n-spin :show="previewBusy" size="small" class="h-full">
              <div class="relative z-10 w-full h-full min-h-52 flex items-center justify-center px-3">
                <n-skeleton v-if="editorStatus === 'loading'" text :repeat="3" class="w-full" />
                <div v-else-if="editorStatus === 'error'" class="text-xs text-rose-400">
                  SVG 加载失败
                </div>
                <div v-else-if="editorPreviewSvg" class="w-full h-full flex items-center justify-center">
                  <div class="preview-scale-wrapper w-full h-full flex items-center justify-center" :class="previewScaleClass">
                    <div class="editor-preview w-full h-full" v-html="editorPreviewSvg" />
                  </div>
                </div>
                <div v-else class="text-xs text-slate-400">
                  请选择图标进行编辑
                </div>
              </div>
            </n-spin>
          </div>

          <div class="flex items-center justify-between text-xs text-slate-400">
            <span class="truncate">{{ activeIcon?.name || '未选择' }}</span>
            <span v-if="activeState">尺寸 {{ activeState.width }} × {{ activeState.height }}</span>
          </div>
        </div>

        <!-- 编辑面板 -->
        <div v-if="editorStatus === 'loading'" class="space-y-4">
          <n-skeleton text :repeat="2" />
          <n-skeleton text :repeat="4" />
        </div>
        <n-collapse v-else v-model:expanded-names="collapseExpanded">
          <n-collapse-item name="appearance" title="外观设置" :disabled="!activeState">
            <div class="rounded-xl border border-slate-200/60 dark:border-white/10 bg-white/70 dark:bg-[#18181b] p-4 space-y-4">
              <div class="flex items-center justify-between">
                <span class="text-xs font-semibold text-slate-400 dark:text-gray-500 uppercase tracking-wider">全局颜色</span>
                <n-switch
                  :value="activeState?.applyColor ?? false"
                  size="small"
                  :disabled="!activeState"
                  @update:value="(value: boolean) => updateActiveState('applyColor', value)"
                />
              </div>
              <n-color-picker
                :value="activeState?.color"
                :swatches="mergedSwatches"
                size="small"
                :disabled="!activeState?.applyColor"
                @update:value="(value: string) => value && updateActiveState('color', value)"
              />
            </div>
          </n-collapse-item>

          <n-collapse-item name="transform" title="尺寸与变换" :disabled="!activeState">
            <div class="rounded-xl border border-slate-200/60 dark:border-white/10 bg-white/70 dark:bg-[#18181b] p-4 space-y-4">
              <div class="grid grid-cols-2 gap-2">
                <n-input-number
                  :value="activeState?.width"
                  size="small"
                  :min="8"
                  :max="512"
                  :disabled="!activeState"
                  @update:value="(value: number | null) => value !== null && updateActiveState('width', value)"
                >
                  <template #prefix>
                    <span class="text-xs text-slate-400">W</span>
                  </template>
                </n-input-number>
                <n-input-number
                  :value="activeState?.height"
                  size="small"
                  :min="8"
                  :max="512"
                  :disabled="!activeState"
                  @update:value="(value: number | null) => value !== null && updateActiveState('height', value)"
                >
                  <template #prefix>
                    <span class="text-xs text-slate-400">H</span>
                  </template>
                </n-input-number>
              </div>

              <div class="flex flex-wrap gap-1.5">
                <n-tag
                  v-for="size in sizePresets"
                  :key="size"
                  checkable
                  size="small"
                  class="cursor-pointer"
                  :checked="activeState?.width === size"
                  @click="applySizePreset(size)"
                >
                  {{ size }}
                </n-tag>
              </div>

              <n-input-number
                :value="activeState?.rotate"
                size="small"
                :min="-180"
                :max="180"
                :disabled="!activeState"
                @update:value="(value: number | null) => value !== null && updateActiveState('rotate', value)"
              >
                <template #prefix>
                  <span class="text-xs text-slate-400">旋转</span>
                </template>
                <template #suffix>
                  <span class="text-xs text-slate-400">°</span>
                </template>
              </n-input-number>

              <div class="flex gap-2">
                <n-button
                  class="flex-1"
                  size="small"
                  :type="activeState?.flipX ? 'primary' : 'default'"
                  :disabled="!activeState"
                  @click="toggleActiveState('flipX')"
                >
                  <template #icon>
                    <div class="i-carbon-flip-horizontal" />
                  </template>
                  水平
                </n-button>
                <n-button
                  class="flex-1"
                  size="small"
                  :type="activeState?.flipY ? 'primary' : 'default'"
                  :disabled="!activeState"
                  @click="toggleActiveState('flipY')"
                >
                  <template #icon>
                    <div class="i-carbon-flip-vertical" />
                  </template>
                  垂直
                </n-button>
              </div>
            </div>
          </n-collapse-item>

          <n-collapse-item name="stroke" title="线条与圆角" :disabled="!activeState">
            <div class="rounded-xl border border-slate-200/60 dark:border-white/10 bg-white/70 dark:bg-[#18181b] p-4 space-y-4">
              <div class="flex items-center justify-between">
                <span class="text-xs text-slate-500 dark:text-gray-400">圆角端点</span>
                <n-switch
                  :value="activeState?.roundStroke ?? false"
                  size="small"
                  :disabled="!activeState"
                  @update:value="(value: boolean) => updateActiveState('roundStroke', value)"
                />
              </div>

              <n-input-number
                :value="activeState?.strokeWidth"
                size="small"
                :min="0"
                :max="24"
                :disabled="!activeState"
                @update:value="(value: number | null) => updateActiveState('strokeWidth', value)"
              >
                <template #prefix>
                  <span class="text-xs text-slate-400">粗细</span>
                </template>
              </n-input-number>

              <n-input-number
                :value="activeState?.rectRadius"
                size="small"
                :min="0"
                :max="32"
                :disabled="!activeState"
                @update:value="(value: number | null) => updateActiveState('rectRadius', value)"
              >
                <template #prefix>
                  <span class="text-xs text-slate-400">圆角</span>
                </template>
              </n-input-number>
            </div>
          </n-collapse-item>

          <n-collapse-item name="element" title="元素级编辑">
            <div class="rounded-xl border border-slate-200/60 dark:border-white/10 bg-white/70 dark:bg-[#18181b] p-4 space-y-4">
              <n-input
                v-model:value="elementSearch"
                size="small"
                clearable
                placeholder="搜索元素（名称/类型）"
                :disabled="!activeElementOptions.length"
              >
                <template #prefix>
                  <div class="i-carbon-search text-slate-400" />
                </template>
              </n-input>

              <n-select
                v-model:value="activeElementKey"
                size="small"
                :options="filteredElementOptions.map(item => ({ label: item.label, value: item.key }))"
                placeholder="选择线条元素"
                :disabled="!activeElementOptions.length"
                virtual-scroll
              />

              <div v-if="activeElementStyle" class="p-3 bg-white/80 dark:bg-[#121214] rounded-lg border border-slate-200/60 dark:border-white/10 space-y-3">
                <div class="flex items-center justify-between">
                  <span class="text-xs font-medium text-slate-600 dark:text-gray-300">独立样式</span>
                  <n-switch
                    :value="activeElementStyle.enabled"
                    size="small"
                    @update:value="(value: boolean) => updateActiveElementStyle('enabled', value)"
                  />
                </div>

                <template v-if="activeElementStyle.enabled">
                  <n-color-picker
                    :value="activeElementStyle.strokeColor"
                    size="small"
                    :swatches="mergedSwatches"
                    @update:value="(value: string) => value && updateActiveElementStyle('strokeColor', value)"
                  />
                  <n-input-number
                    :value="activeElementStyle.strokeWidth"
                    size="small"
                    :min="0"
                    :step="0.5"
                    @update:value="(value: number | null) => updateActiveElementStyle('strokeWidth', value)"
                  >
                    <template #prefix>
                      <span class="text-xs">粗细</span>
                    </template>
                  </n-input-number>
                  <div class="flex items-center justify-between">
                    <span class="text-xs text-slate-500">圆角</span>
                    <n-switch
                      :value="activeElementStyle.roundStroke"
                      size="small"
                      @update:value="(value: boolean) => updateActiveElementStyle('roundStroke', value)"
                    />
                  </div>
                  <div class="flex justify-end">
                    <n-button size="tiny" secondary @click="resetActiveElementStyle">
                      重置当前元素
                    </n-button>
                  </div>
                </template>
              </div>

              <div v-else-if="!activeElementOptions.length" class="text-xs text-slate-400 text-center py-2">
                此图标无可编辑元素
              </div>
            </div>
          </n-collapse-item>
        </n-collapse>
      </div>
    </n-scrollbar>

    <!-- 底部操作栏 -->
    <div class="flex-shrink-0 p-4 border-t border-slate-200/70 dark:border-white/5 bg-slate-50/80 dark:bg-[#252529] space-y-3">
      <div class="flex flex-wrap items-center gap-3">
        <div class="flex items-center gap-2">
          <span class="text-xs text-slate-500">保存格式</span>
          <n-radio-group v-model:value="saveFormat" size="small">
            <n-radio-button value="svg">
              SVG
            </n-radio-button>
            <n-radio-button value="png">
              PNG
            </n-radio-button>
            <n-radio-button value="both">
              Both
            </n-radio-button>
          </n-radio-group>
        </div>
        <div v-if="needsPngSize" class="flex flex-wrap items-center gap-2">
          <span class="text-xs text-slate-500">PNG 尺寸</span>
          <n-input-number
            :value="pngSize"
            size="small"
            :min="16"
            :max="1024"
            :step="2"
            @update:value="updatePngSize"
          />
          <div class="flex flex-wrap gap-1">
            <n-tag
              v-for="size in pngSizePresets"
              :key="size"
              checkable
              size="small"
              class="cursor-pointer"
              :checked="pngSize === size"
              @click="pngSize = size"
            >
              {{ size }}
            </n-tag>
          </div>
        </div>
      </div>

      <div class="flex gap-2">
        <n-input
          v-model:value="savePath"
          size="small"
          placeholder="保存目录"
          class="flex-1"
        >
          <template #prefix>
            <div class="i-carbon-folder text-gray-400" />
          </template>
        </n-input>
        <n-button size="small" secondary @click="selectDirectory">
          ...
        </n-button>
      </div>

      <div class="grid grid-cols-3 gap-2">
        <n-button size="small" secondary :disabled="!editorPreviewSvg" @click="copyEditedSvg">
          <template #icon>
            <div class="i-carbon-copy" />
          </template>
          复制
        </n-button>
        <n-button size="small" secondary :disabled="!activeState" @click="resetActiveEditor">
          复原
        </n-button>
        <n-button size="small" type="primary" :disabled="!activeIcon || disabled" @click="handleSave">
          保存
        </n-button>
      </div>
    </div>
  </div>
</template>

<style scoped>
/* 编辑器预览放大与选中高亮 */
.editor-preview :deep(svg) {
  width: 100%;
  height: 100%;
  max-width: 100%;
  max-height: 100%;
}

.editor-preview :deep([data-editor-focus='true']) {
  filter: drop-shadow(0 0 6px rgba(126, 156, 180, 0.6));
}

/* 预览缩放（避免依赖预设类缺失） */
.preview-scale-wrapper {
  transform-origin: center;
}
.preview-scale-50 {
  transform: scale(0.5);
}
.preview-scale-75 {
  transform: scale(0.75);
}
.preview-scale-100 {
  transform: scale(1);
}
.preview-scale-125 {
  transform: scale(1.25);
}
.preview-scale-150 {
  transform: scale(1.5);
}
.preview-scale-200 {
  transform: scale(2);
}

/* 网格背景图案 */
.pattern-grid {
  background-image:
    linear-gradient(to right, rgba(255, 255, 255, 0.03) 1px, transparent 1px),
    linear-gradient(to bottom, rgba(255, 255, 255, 0.03) 1px, transparent 1px);
  background-size: 16px 16px;
}
</style>
