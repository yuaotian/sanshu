<script setup lang="ts">
import { invoke } from '@tauri-apps/api/core'
import { computed, onMounted, ref, watch } from 'vue'

// 类型定义
interface AudioAsset {
  id: string
  name: string
  filename: string
}

const props = defineProps({
  audioNotificationEnabled: {
    type: Boolean,
    required: true,
  },
  audioUrl: {
    type: String,
    default: '',
  },
})

const emit = defineEmits(['toggleAudioNotification', 'updateAudioUrl', 'testAudio', 'stopAudio', 'testAudioError'])

// 预设音效配置（从后端获取）
const presetSounds = ref<AudioAsset[]>([])
const loading = ref(true)

// 当前选中的音效类型
const selectedSoundType = ref('preset') // 'preset' | 'custom'
const selectedPreset = ref('default') // 使用资源ID
const customUrl = ref('')
const customName = ref('')

// 自定义音频状态
const customAudioState = ref('hidden') // 'hidden' | 'show_input' | 'saved' | 'verified'
const isTestingCustom = ref(false)

// 加载可用音频资源
async function loadAudioAssets() {
  try {
    loading.value = true
    const assets = await invoke('get_available_audio_assets') as AudioAsset[]
    presetSounds.value = assets.map(asset => ({
      id: asset.id,
      name: asset.name,
      filename: asset.filename,
    }))

    // 如果没有选中的预设，默认选择第一个
    if (!selectedPreset.value && presetSounds.value.length > 0) {
      selectedPreset.value = presetSounds.value[0].id
    }
  }
  catch (error) {
    console.error('加载音频资源失败:', error)
  }
  finally {
    loading.value = false
  }
}

// 计算当前音效URL
const currentAudioUrl = computed(() => {
  if (selectedSoundType.value === 'preset') {
    return selectedPreset.value // 返回资源ID
  }
  else {
    return customUrl.value
  }
})

// 更新音效配置
function updateAudioConfig() {
  emit('updateAudioUrl', currentAudioUrl.value)
}

// 切换到预设音效
function selectPreset(presetId) {
  selectedSoundType.value = 'preset'
  selectedPreset.value = presetId
  updateAudioConfig()
  // 停止之前的播放并播放新音效
  stopPreviousAudio()
  emit('testAudio')
}

// 停止之前的音频播放
function stopPreviousAudio() {
  // 发送停止音频事件到父组件
  emit('stopAudio')
}

// 切换到自定义音效
function selectCustom() {
  // 如果已经验证，直接切换到自定义模式
  if (customAudioState.value === 'verified') {
    selectedSoundType.value = 'custom'
    updateAudioConfig()
  }
  else {
    // 否则显示自定义输入界面，设置为显示状态
    customAudioState.value = 'show_input'
    if (!customUrl.value) {
      customUrl.value = ''
      customName.value = ''
    }
  }
}

// 判断是否为预设音效ID（从服务端获取的列表）
function isPresetId(id: string) {
  return presetSounds.value.some(s => s.id === id)
}

// 保存自定义URL
async function saveCustomUrl() {
  if (!customUrl.value.trim())
    return

  customAudioState.value = 'saved'
  // 这里不立即切换到自定义模式，需要试听成功后才切换
}

// 试听自定义音频
async function testCustomAudio() {
  if (!customUrl.value.trim())
    return

  isTestingCustom.value = true

  // 保存原来的URL，以便失败时恢复
  const originalUrl = props.audioUrl

  try {
    // 临时设置自定义URL进行试听
    emit('updateAudioUrl', customUrl.value)

    // 直接调用后端测试音频，而不是通过事件
    await invoke('test_audio_sound')

    // 试听成功，标记为已验证并切换到自定义模式
    customAudioState.value = 'verified'
    selectedSoundType.value = 'custom'

    console.log('✅ 自定义音频试听成功，已切换到自定义模式')
  }
  catch (error) {
    // 试听失败，恢复原来的URL
    emit('updateAudioUrl', originalUrl)
    customAudioState.value = 'show_input'
    console.error('❌ 自定义音频试听失败:', error)

    // 显示错误提示给用户
    // 这里可以通过事件通知父组件显示错误消息
    emit('testAudioError', error)
  }
  finally {
    isTestingCustom.value = false
  }
}

// 更新自定义URL输入
function updateCustomUrl() {
  // 输入时重置状态
  if (customAudioState.value !== 'verified') {
    customAudioState.value = 'show_input'
  }
}

// 初始化当前状态
function initializeState() {
  if (isPresetId(props.audioUrl)) {
    selectedSoundType.value = 'preset'
    customAudioState.value = 'hidden'
    selectedPreset.value = props.audioUrl
    // 清空自定义URL，避免显示预设ID
    customUrl.value = ''
    customName.value = ''
  }
  else if (props.audioUrl) {
    selectedSoundType.value = 'custom'
    customUrl.value = props.audioUrl
    customAudioState.value = 'verified' // 已有自定义URL说明已验证
  }
  else {
    // 默认状态
    selectedSoundType.value = 'preset'
    customAudioState.value = 'hidden'
    customUrl.value = ''
    customName.value = ''
    // 如果没有设置，使用第一个可用的预设
    if (presetSounds.value.length > 0) {
      selectedPreset.value = presetSounds.value[0].id
    }
  }
}

// 监听props变化
watch(() => props.audioUrl, () => {
  initializeState()
}, { immediate: true })

onMounted(async () => {
  await loadAudioAssets()
  initializeState()
})
</script>

<template>
  <!-- 设置内容 -->
  <n-space vertical size="large">
    <!-- 音频通知开关 -->
    <div class="flex items-center justify-between">
      <div class="flex items-center">
        <div class="w-1.5 h-1.5 bg-warning rounded-full mr-3 flex-shrink-0" />
        <div>
          <div class="text-sm font-medium leading-relaxed">
            音频通知
          </div>
          <div class="text-xs opacity-60">
            启用后在MCP工具被触发时播放音频提示
          </div>
        </div>
      </div>
      <n-switch
        :value="audioNotificationEnabled"
        size="small"
        @update:value="$emit('toggleAudioNotification')"
      />
    </div>

    <!-- 音效选择 -->
    <div v-if="audioNotificationEnabled" class="pt-4 border-t border-border">
      <div class="flex items-start">
        <div class="w-1.5 h-1.5 bg-warning rounded-full mr-3 mt-2 flex-shrink-0" />
        <div class="flex-1">
          <div class="text-sm font-medium mb-3 leading-relaxed">
            音效选择
          </div>

          <!-- 预设音效 -->
          <div class="mb-4">
            <div class="text-xs opacity-60 mb-2">
              预设音效
            </div>
            <div v-if="loading" class="text-xs opacity-60">
              加载中...
            </div>
            <n-space v-else>
              <n-button
                v-for="preset in presetSounds"
                :key="preset.id"
                :type="selectedSoundType === 'preset' && selectedPreset === preset.id ? 'primary' : 'default'"
                size="small"
                @click="selectPreset(preset.id)"
              >
                {{ preset.name }}
              </n-button>
            </n-space>
          </div>

          <!-- 自定义音效 -->
          <div class="mb-3">
            <div class="flex items-center justify-between mb-2">
              <div class="text-xs opacity-60">
                自定义音效
              </div>
              <n-button
                :type="selectedSoundType === 'custom' ? 'primary' : 'default'"
                size="tiny"
                @click="selectCustom"
              >
                使用自定义
              </n-button>
            </div>

            <div v-if="selectedSoundType === 'custom' || customAudioState !== 'hidden'" class="space-y-2">
              <!-- 自定义名称 -->
              <n-input
                v-model:value="customName"
                size="small"
                placeholder="音效名称（可选）"
              />

              <!-- 自定义URL -->
              <n-input-group>
                <n-input
                  v-model:value="customUrl"
                  size="small"
                  placeholder="音效文件路径或URL"
                  @input="updateCustomUrl"
                />
                <n-button
                  v-if="customAudioState === 'show_input'"
                  type="primary"
                  size="small"
                  :disabled="!customUrl.trim()"
                  @click="saveCustomUrl"
                >
                  <template #icon>
                    <div class="i-carbon-save text-sm" />
                  </template>
                  保存
                </n-button>
                <n-button
                  v-else-if="customAudioState === 'saved'"
                  type="success"
                  size="small"
                  :disabled="!customUrl.trim() || isTestingCustom"
                  :loading="isTestingCustom"
                  @click="testCustomAudio"
                >
                  <template #icon>
                    <div class="i-carbon-volume-up text-sm" />
                  </template>
                  试听
                </n-button>
                <n-button
                  v-else
                  type="info"
                  size="small"
                  :disabled="!customUrl.trim()"
                  @click="$emit('testAudio')"
                >
                  <template #icon>
                    <div class="i-carbon-volume-up text-sm" />
                  </template>
                  试听
                </n-button>
              </n-input-group>

              <div class="text-xs opacity-60">
                <span v-if="customAudioState === 'show_input'">
                  示例：/path/to/sound.mp3 或 https://example.com/notification.wav
                </span>
                <span v-else-if="customAudioState === 'saved'" class="text-warning">
                  请点击试听按钮测试音频，试听成功后将自动切换到自定义音效
                </span>
                <span v-else class="text-success">
                  ✅ 自定义音效已验证并启用
                </span>
              </div>
            </div>
          </div>

          <n-card size="small" embedded class="mt-3">
            <div class="text-xs">
              <span class="opacity-60">当前音效：</span>
              <span v-if="selectedSoundType === 'preset'">
                {{ presetSounds.find(p => p.id === selectedPreset)?.name }}
              </span>
              <span v-else-if="customUrl">
                {{ customName || '自定义音效' }} ({{ customUrl }})
              </span>
              <span v-else class="opacity-60">未设置</span>
            </div>
          </n-card>
        </div>
      </div>
    </div>
  </n-space>
</template>
