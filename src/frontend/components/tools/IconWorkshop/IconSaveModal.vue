<script setup lang="ts">
/**
 * 图标保存模态框组件 - 重构版
 * 提供沉浸式的保存配置和预览体验
 */
import { invoke } from '@tauri-apps/api/core'
import { computed, ref, watch } from 'vue'
import type { IconFormat, IconItem, IconSaveRequest } from '../../../types/icon'
import { sanitizeSvg } from '../../../utils/sanitize'
import AppModal from '../../common/AppModal.vue'

// Props
interface Props {
  show: boolean
  icons: IconItem[]
  defaultPath?: string
}

const props = withDefaults(defineProps<Props>(), {
  defaultPath: 'assets/icons',
})

// Emits
const emit = defineEmits<{
  'update:show': [value: boolean]
  'save': [request: IconSaveRequest]
}>()

// 本地状态
const savePath = ref(props.defaultPath)
const format = ref<IconFormat>('svg')
const saving = ref(false)

// 格式选项配置
const formatOptions = [
  { 
    label: 'SVG 矢量', 
    value: 'svg', 
    desc: '保留原始矢量数据，可无限缩放',
    icon: 'i-carbon-vector-pen'
  },
  { 
    label: 'PNG 位图', 
    value: 'png', 
    desc: '标清位图，兼容性好',
    icon: 'i-carbon-image'
  },
  { 
    label: '双格式', 
    value: 'both', 
    desc: '同时保存 SVG 和 PNG 版本',
    icon: 'i-carbon-copy-file'
  },
] as const

// 监听默认路径变化
watch(() => props.defaultPath, (newPath) => {
  if (newPath) {
    savePath.value = newPath
  }
})

// 计算属性
const dialogVisible = computed({
  get: () => props.show,
  set: (value: boolean) => emit('update:show', value),
})

const iconCount = computed(() => props.icons.length)

function processSvg(content?: string) {
  if (!content) return null
  return sanitizeSvg(content
    .replace(/\s*style="[^"]*"/g, '')
    .replace(/\s*width="[^"]*"/g, ' width="100%"')
    .replace(/\s*height="[^"]*"/g, ' height="100%"'))
}

// 选择目录
async function selectDirectory() {
  try {
    const result = await invoke<string | null>('select_icon_save_directory', {
      defaultPath: savePath.value,
    })
    if (result) {
      savePath.value = result
    }
  }
  catch (e) {
    console.error('选择目录失败:', e)
  }
}

// 执行保存
async function handleSave() {
  if (!savePath.value.trim()) return

  saving.value = true
  try {
    emit('save', {
      icons: props.icons,
      savePath: savePath.value,
      format: format.value,
    })
  }
  finally {
    saving.value = false
  }
}

// 取消
function handleCancel() {
  dialogVisible.value = false
}
</script>

<template>
  <AppModal
    v-model:show="dialogVisible"
    title="保存图标"
    width="760px"
    :mask-closable="!saving"
    :close-on-esc="!saving"
  >
    <div class="flex flex-col md:flex-row gap-6">
      <!-- 左侧：配置面板 -->
      <div class="w-full md:w-[300px] flex flex-col gap-5 flex-shrink-0">
        <p class="text-sm text-on-surface-secondary">配置导出选项和目标路径</p>

        <!-- 路径选择 -->
        <div class="flex flex-col gap-2">
          <label class="text-xs font-semibold uppercase tracking-wider text-on-surface-muted">保存路径</label>
          <div class="flex gap-2">
            <n-input
              v-model:value="savePath"
              size="small"
              placeholder="选择目录..."
              class="flex-1"
            >
              <template #prefix>
                <div class="i-carbon-folder text-on-surface-muted" />
              </template>
            </n-input>
            <n-button secondary type="primary" @click="selectDirectory">
              <template #icon>
                <div class="i-carbon-folder-open" />
              </template>
            </n-button>
          </div>
        </div>

        <!-- 格式选择 -->
        <div class="flex flex-col gap-3">
          <label class="text-xs font-semibold uppercase tracking-wider text-on-surface-muted">导出格式</label>
          <div class="flex flex-col gap-2">
            <div
              v-for="opt in formatOptions"
              :key="opt.value"
              class="relative px-4 py-3 rounded-lg border-2 cursor-pointer transition-all duration-200 group flex items-center gap-3"
              :class="[
                format === opt.value
                  ? 'border-primary-500 bg-primary-500/5'
                  : 'border-transparent bg-container hover:border-border'
              ]"
              @click="format = opt.value as IconFormat"
            >
              <div v-if="format === opt.value" class="absolute right-2 top-2 text-primary">
                <div class="i-carbon-checkmark-filled text-lg" />
              </div>

              <div
                class="w-10 h-10 rounded-full flex items-center justify-center text-xl transition-colors"
                :class="format === opt.value ? 'bg-primary-500/10 text-primary' : 'bg-container-secondary text-on-surface-muted'"
              >
                <div :class="opt.icon" />
              </div>

              <div class="flex-1">
                <div class="font-medium text-on-surface" :class="{ 'text-primary': format === opt.value }">
                  {{ opt.label }}
                </div>
                <div class="text-xs text-on-surface-muted leading-tight mt-0.5">{{ opt.desc }}</div>
              </div>
            </div>
          </div>
        </div>

        <!-- 底部按钮 -->
        <div class="mt-auto pt-4 border-t border-border flex flex-col gap-2">
          <n-button
            type="primary"
            size="small"
            block
            :loading="saving"
            :disabled="!savePath.trim()"
            @click="handleSave"
          >
            <template #icon>
              <div class="i-carbon-download" />
            </template>
            确认保存 ({{ iconCount }})
          </n-button>
          <n-button quaternary size="small" block @click="handleCancel">
            取消
          </n-button>
        </div>
      </div>

      <!-- 右侧：预览面板 -->
      <div class="flex-1 bg-container rounded-lg flex flex-col overflow-hidden border border-border">
        <div class="px-4 py-3 flex justify-between items-center border-b border-border">
          <div>
            <div class="text-sm font-medium text-on-surface">预览清单</div>
            <div class="text-xs text-on-surface-muted mt-0.5">即将保存 {{ iconCount }} 个图标</div>
          </div>
          <n-tag size="small" :bordered="false" type="info">SVG</n-tag>
        </div>

        <div class="flex-1 overflow-y-auto p-4">
          <div class="grid grid-cols-3 sm:grid-cols-4 gap-3">
            <div
              v-for="icon in icons"
              :key="icon.id"
              class="group relative aspect-square rounded-lg bg-surface border border-border hover:border-primary-400 transition-all duration-200 flex flex-col items-center justify-center p-3"
            >
              <div class="flex-1 w-full flex items-center justify-center text-on-surface group-hover:text-primary transition-colors">
                <div
                  v-if="icon.svgContent"
                  class="w-8 h-8 md:w-10 md:h-10 transition-transform duration-200 group-hover:scale-110"
                  v-html="processSvg(icon.svgContent)"
                />
                <div v-else class="i-carbon-image text-4xl opacity-20" />
              </div>
              <div class="w-full text-center mt-2">
                <div class="text-xs text-on-surface-secondary group-hover:text-on-surface truncate transition-colors">
                  {{ icon.name }}
                </div>
              </div>
            </div>
          </div>

          <div v-if="icons.length === 0" class="h-full flex flex-col items-center justify-center text-on-surface-disabled">
            <div class="i-carbon-select-window text-6xl mb-4" />
            <p>未选择图标</p>
          </div>
        </div>
      </div>
    </div>
  </AppModal>
</template>

