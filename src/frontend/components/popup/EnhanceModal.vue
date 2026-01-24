<script setup lang="ts">
import { invoke } from '@tauri-apps/api/core'
import { listen, type UnlistenFn } from '@tauri-apps/api/event'
import { useMessage } from 'naive-ui'
import { computed, onMounted, onUnmounted, ref, watch } from 'vue'

interface Props {
  show: boolean
  originalPrompt: string
  projectRootPath?: string
  currentFilePath?: string
}

interface Emits {
  'update:show': [value: boolean]
  confirm: [enhancedPrompt: string]
  cancel: []
}

interface EnhanceStreamEvent {
  event_type: 'chunk' | 'complete' | 'error'
  chunk?: string
  accumulated_text?: string
  enhanced_prompt?: string
  error?: string
  progress: number
}

const props = defineProps<Props>()
const emit = defineEmits<Emits>()

const message = useMessage()

// 状态
const isEnhancing = ref(false)
const streamContent = ref('')
const enhancedPrompt = ref('')
const progress = ref(0)
const errorMessage = ref('')
const hasCompleted = ref(false)

// 事件监听器
let unlisten: UnlistenFn | null = null

// 计算属性
const canConfirm = computed(() => hasCompleted.value && enhancedPrompt.value.length > 0)
const statusText = computed(() => {
  if (errorMessage.value) return '增强失败'
  if (hasCompleted.value) return '增强完成'
  if (isEnhancing.value) return `增强中... ${progress.value}%`
  return '准备就绪'
})

// 开始增强
async function startEnhance() {
  if (isEnhancing.value) return

  // 启动前释放旧监听，避免重复订阅导致多次回调
  if (unlisten) {
    unlisten()
    unlisten = null
  }

  // 重置状态
  isEnhancing.value = true
  streamContent.value = ''
  enhancedPrompt.value = ''
  progress.value = 0
  errorMessage.value = ''
  hasCompleted.value = false

  try {
    // 设置事件监听
    unlisten = await listen<EnhanceStreamEvent>('enhance-stream', (event) => {
      const data = event.payload
      
      switch (data.event_type) {
        case 'chunk':
          if (data.accumulated_text) {
            streamContent.value = data.accumulated_text
          }
          progress.value = data.progress
          break
        case 'complete':
          if (data.enhanced_prompt) {
            enhancedPrompt.value = data.enhanced_prompt
          }
          if (data.accumulated_text) {
            streamContent.value = data.accumulated_text
          }
          progress.value = 100
          hasCompleted.value = true
          isEnhancing.value = false
          break
        case 'error':
          errorMessage.value = data.error || '未知错误'
          isEnhancing.value = false
          break
      }
    })

    // 调用后端增强
    await invoke('enhance_prompt_stream', {
      prompt: props.originalPrompt,
      projectRootPath: props.projectRootPath || null,
      currentFilePath: props.currentFilePath || null,
      includeHistory: true,
    })
  }
  catch (error) {
    console.error('增强失败:', error)
    errorMessage.value = String(error)
    isEnhancing.value = false
  }
}

// 确认使用增强结果
function handleConfirm() {
  if (!canConfirm.value) return
  emit('confirm', enhancedPrompt.value)
  handleClose()
}

// 重试增强
function handleRetry() {
  if (isEnhancing.value) return
  startEnhance()
}

// 取消/关闭
function handleClose() {
  emit('update:show', false)
  emit('cancel')
  cleanup()
}

// 清理
function cleanup() {
  if (unlisten) {
    unlisten()
    unlisten = null
  }
  isEnhancing.value = false
  streamContent.value = ''
  enhancedPrompt.value = ''
  progress.value = 0
  errorMessage.value = ''
  hasCompleted.value = false
}

// 监听 show 变化，自动开始增强
watch(() => props.show, (newValue) => {
  if (newValue && props.originalPrompt) {
    startEnhance()
  }
  // 关闭弹窗时清理监听与状态，避免残留
  if (!newValue) {
    cleanup()
  }
})

// 组件挂载
onMounted(() => {
  if (props.show && props.originalPrompt) {
    startEnhance()
  }
})

// 组件卸载时清理
onUnmounted(() => {
  cleanup()
})
</script>

<template>
  <n-modal
    :show="show"
    preset="card"
    :title="'✨ 提示词增强'"
    :closable="!isEnhancing"
    :mask-closable="!isEnhancing"
    style="width: 600px; max-width: 90vw;"
    @update:show="(val: boolean) => !isEnhancing && emit('update:show', val)"
  >
    <!-- 原始提示词 -->
    <div class="mb-4">
      <div class="text-xs text-gray-400 mb-1 flex items-center gap-1">
        <div class="i-carbon-document w-3 h-3" />
        原始提示词
      </div>
      <div class="p-3 bg-gray-800/50 rounded-lg text-sm text-gray-300 max-h-24 overflow-y-auto">
        {{ originalPrompt }}
      </div>
    </div>

    <!-- 进度条 -->
    <div v-if="isEnhancing" class="mb-4">
      <div class="flex items-center justify-between text-xs text-gray-400 mb-1">
        <span>{{ statusText }}</span>
        <span>{{ progress }}%</span>
      </div>
      <n-progress
        type="line"
        :percentage="progress"
        :height="6"
        :border-radius="3"
        :show-indicator="false"
        status="info"
      />
    </div>

    <!-- 流式输出显示区域 -->
    <div class="mb-4">
      <div class="text-xs text-gray-400 mb-1 flex items-center gap-1">
        <div class="i-carbon-magic-wand w-3 h-3" />
        {{ hasCompleted ? '增强结果' : '实时输出' }}
      </div>
      <div
        class="p-3 bg-gray-900 rounded-lg text-sm min-h-32 max-h-64 overflow-y-auto border transition-colors"
        :class="[
          hasCompleted ? 'border-green-500/50' : 'border-gray-700',
          errorMessage ? 'border-red-500/50' : ''
        ]"
      >
        <!-- 增强完成后显示增强结果 -->
        <div v-if="hasCompleted && enhancedPrompt" class="text-green-400 whitespace-pre-wrap">
          {{ enhancedPrompt }}
        </div>
        <!-- 增强中显示流式内容 -->
        <div v-else-if="streamContent" class="text-gray-300 whitespace-pre-wrap">
          {{ streamContent }}
          <span v-if="isEnhancing" class="inline-block w-2 h-4 bg-blue-500 animate-pulse ml-1" />
        </div>
        <!-- 错误信息 -->
        <div v-else-if="errorMessage" class="text-red-400">
          ❌ {{ errorMessage }}
        </div>
        <!-- 等待中 -->
        <div v-else class="text-gray-500 flex items-center gap-2">
          <n-spin size="small" />
          正在准备增强...
        </div>
      </div>
    </div>

    <!-- 操作按钮 -->
    <div class="flex justify-end gap-3">
      <n-button
        @click="handleClose"
      >
        取消
      </n-button>
      <n-button
        v-if="errorMessage && !isEnhancing"
        type="warning"
        @click="handleRetry"
      >
        重试
      </n-button>
      <n-button
        type="primary"
        :disabled="!canConfirm"
        @click="handleConfirm"
      >
        <template #icon>
          <div class="i-carbon-checkmark w-4 h-4" />
        </template>
        使用增强结果
      </n-button>
    </div>
  </n-modal>
</template>

<style scoped>
/* 光标闪烁动画 */
@keyframes blink {
  0%, 50% { opacity: 1; }
  51%, 100% { opacity: 0; }
}

.animate-pulse {
  animation: blink 1s ease-in-out infinite;
}
</style>
