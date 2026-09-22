<script setup lang="ts">
import type { McpRequest } from '../../types/popup'
import { computed, onMounted } from 'vue'
import { useShortcuts } from '../../composables/useShortcuts'

interface Props {
  request: McpRequest | null
  loading?: boolean
  submitting?: boolean
  canSubmit?: boolean
  canEnhance?: boolean
  connectionStatus?: string
  continueReplyEnabled?: boolean
  inputStatusText?: string
  enhanceEnabled?: boolean
  // 中文注释：已启用且可用于增强的 MCP 工具名称列表
  enhanceToolNames?: string[]
}

interface Emits {
  submit: []
  continue: []
  enhance: []
  openMcpToolsTab: []
}

const props = withDefaults(defineProps<Props>(), {
  loading: false,
  submitting: false,
  canSubmit: false,
  canEnhance: false,
  connectionStatus: '已连接',
  continueReplyEnabled: true,
  inputStatusText: '',
  enhanceEnabled: false,
  enhanceToolNames: () => [],
})

const emit = defineEmits<Emits>()

// 使用自定义快捷键系统
const {
  quickSubmitShortcutText,
  enhanceShortcutText,
  continueShortcutText,
  useQuickSubmitShortcut,
  useEnhanceShortcut,
  useContinueShortcut,
  loadShortcutConfig,
} = useShortcuts()

const shortcutText = quickSubmitShortcutText

// 中文注释：增强按钮的工具辅助提示文本
const enhanceToolHint = computed(() => {
  if (!props.enhanceToolNames || props.enhanceToolNames.length === 0)
    return ''
  return `增强时将引导 AI 使用 ${props.enhanceToolNames.join('、')} 辅助`
})

const statusText = computed(() => {
  // 如果可以提交，直接显示快捷键提示
  if (props.canSubmit) {
    return shortcutText.value
  }

  // 如果有输入状态文本且不是默认状态，显示输入状态
  if (props.inputStatusText && props.inputStatusText !== '等待输入...') {
    return props.inputStatusText
  }

  // 根据请求类型显示不同的提示
  if (props.request?.predefined_options) {
    return '选择选项或输入文本'
  }
  return '请输入内容'
})

// 处理快捷键
useQuickSubmitShortcut(() => {
  if (props.canSubmit && !props.submitting) {
    handleSubmit()
  }
})

useEnhanceShortcut(() => {
  if (!props.submitting && props.canEnhance) {
    handleEnhance()
  }
})

useContinueShortcut(() => {
  if (!props.submitting) {
    handleContinue()
  }
})

function handleSubmit() {
  if (props.canSubmit && !props.submitting) {
    emit('submit')
  }
}

function handleContinue() {
  if (!props.submitting) {
    emit('continue')
  }
}

function handleEnhance() {
  if (!props.submitting) {
    if (props.enhanceEnabled && props.canEnhance) {
      emit('enhance')
    }
    else {
      emit('openMcpToolsTab')
    }
  }
}

// 组件挂载时加载快捷键配置
onMounted(() => {
  loadShortcutConfig()
})
</script>

<template>
  <div class="px-4 py-3 bg-gray-100 min-h-[60px] select-none">
    <div v-if="!loading" class="flex justify-between items-center">
      <!-- 左侧状态信息 -->
      <div class="flex items-center">
        <div class="flex items-center gap-2 text-xs text-gray-600">
          <div class="w-2 h-2 rounded-full bg-primary-500" />
          <span class="font-medium">{{ connectionStatus }}</span>
          <span class="opacity-60">|</span>
          <span class="opacity-60">{{ statusText }}</span>
        </div>
      </div>

      <!-- 右侧操作按钮 -->
      <!-- 中文注释：主次分明——「发送」是唯一的实心主按钮且尺寸最大，
           「继续」「本地增强」降为次级样式与 small 尺寸，避免与主操作争夺注意力 -->
      <div class="flex items-center" data-guide="popup-actions">
        <n-space size="small" align="center">
          <!-- 增强按钮 / 启用 CTA -->
          <n-tooltip v-if="enhanceEnabled" trigger="hover" placement="top">
            <template #trigger>
              <n-button
                :disabled="!canEnhance || submitting"
                size="small"
                tertiary
                data-guide="enhance-button"
                @click="handleEnhance"
              >
                <template #icon>
                  <div class="i-carbon-magic-wand w-4 h-4" />
                </template>
                本地增强
              </n-button>
            </template>
            <div>
              <div>{{ canEnhance ? enhanceShortcutText : '请先输入要增强的文本' }}</div>
              <div v-if="enhanceToolHint && canEnhance" class="mt-1 text-xs opacity-75">
                {{ enhanceToolHint }}
              </div>
            </div>
          </n-tooltip>
          <n-tooltip v-else trigger="hover" placement="top">
            <template #trigger>
              <n-button
                :disabled="submitting"
                size="small"
                type="warning"
                secondary
                data-guide="enhance-cta"
                @click="handleEnhance"
              >
                <template #icon>
                  <div class="i-carbon-launch w-4 h-4" />
                </template>
                启用增强
              </n-button>
            </template>
            前往 MCP 工具启用提示词增强
          </n-tooltip>

          <!-- 继续按钮 -->
          <n-tooltip v-if="continueReplyEnabled" trigger="hover" placement="top">
            <template #trigger>
              <n-button
                :disabled="submitting"
                :loading="submitting"
                size="small"
                secondary
                data-guide="continue-button"
                @click="handleContinue"
              >
                <template #icon>
                  <div class="i-carbon-play w-4 h-4" />
                </template>
                继续
              </n-button>
            </template>
            {{ continueShortcutText }}
          </n-tooltip>

          <!-- 发送按钮 -->
          <n-tooltip trigger="hover" placement="top">
            <template #trigger>
              <n-button
                type="primary"
                :disabled="!canSubmit || submitting"
                :loading="submitting"
                size="medium"
                data-guide="submit-button"
                @click="handleSubmit"
              >
                <template #icon>
                  <div v-if="!submitting" class="i-carbon-send w-4 h-4" />
                </template>
                {{ submitting ? '发送中...' : '发送' }}
              </n-button>
            </template>
            {{ shortcutText }}
          </n-tooltip>
        </n-space>
      </div>
    </div>
  </div>
</template>
