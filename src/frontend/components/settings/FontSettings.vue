<script setup lang="ts">
import { useMessage } from 'naive-ui'
import { computed, onMounted, ref } from 'vue'
import { useFontManager } from '../../composables/useFontManager'

const message = useMessage()
const {
  fontConfig,
  fontFamilyOptions,
  fontSizeOptions,
  currentFontFamily,
  currentFontScale,
  loadFontConfig,
  loadFontOptions,
  setFontFamily,
  setFontSize,
  setCustomFontFamily,
  resetFontConfig,
} = useFontManager()

// 本地状态
const customFontInput = ref('')
const fontNameInput = ref('')
const isLoading = ref(false)

// 计算当前字体系列的显示名称
const currentFontFamilyName = computed(() => {
  const option = fontFamilyOptions.value.find(opt => opt.id === fontConfig.value.font_family)
  return option?.name || '未知'
})

// 计算当前字体大小的显示名称
const currentFontSizeName = computed(() => {
  const option = fontSizeOptions.value.find(opt => opt.id === fontConfig.value.font_size)
  return option?.name || '未知'
})

// 处理字体系列变更
async function handleFontFamilyChange(value: string) {
  if (isLoading.value)
    return

  try {
    isLoading.value = true
    await setFontFamily(value)

    // 如果切换到非自定义字体，清空自定义字体输入
    if (value !== 'custom') {
      customFontInput.value = ''
    }
    else {
      // 如果切换到自定义字体，使用当前的自定义字体值
      customFontInput.value = fontConfig.value.custom_font_family
    }

    message.success('字体系列已更新')
  }
  catch (error) {
    message.error('更新字体系列失败')
    console.error(error)
  }
  finally {
    isLoading.value = false
  }
}

// 处理字体大小变更
async function handleFontSizeChange(value: string) {
  if (isLoading.value)
    return

  try {
    isLoading.value = true
    await setFontSize(value)
    message.success('字体大小已更新')
  }
  catch (error) {
    message.error('更新字体大小失败')
    console.error(error)
  }
  finally {
    isLoading.value = false
  }
}

// 处理自定义字体系列变更
async function handleCustomFontFamilyChange() {
  if (isLoading.value || !customFontInput.value.trim())
    return

  try {
    isLoading.value = true
    await setCustomFontFamily(customFontInput.value.trim())
    message.success('自定义字体系列已更新')
  }
  catch (error) {
    message.error('更新自定义字体系列失败')
    console.error(error)
  }
  finally {
    isLoading.value = false
  }
}

// 处理字体名称输入应用
async function handleFontNameApply() {
  if (isLoading.value || !fontNameInput.value.trim())
    return

  const fontName = fontNameInput.value.trim()
  // 构建字体CSS值，包含降级字体
  const fontCssValue = `"${fontName}", -apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, sans-serif`

  try {
    isLoading.value = true
    // 先设置为自定义字体模式
    await setFontFamily('custom')
    // 然后设置自定义字体值
    await setCustomFontFamily(fontCssValue)
    customFontInput.value = fontCssValue
    message.success(`字体 "${fontName}" 已应用`)
  }
  catch (error) {
    message.error('应用字体失败')
    console.error(error)
  }
  finally {
    isLoading.value = false
  }
}

// 处理重置配置
async function handleResetConfig() {
  if (isLoading.value)
    return

  try {
    isLoading.value = true
    await resetFontConfig()
    customFontInput.value = fontConfig.value.custom_font_family
    message.success('字体配置已重置')
  }
  catch (error) {
    message.error('重置字体配置失败')
    console.error(error)
  }
  finally {
    isLoading.value = false
  }
}

// 组件挂载时加载数据
onMounted(async () => {
  try {
    await Promise.all([
      loadFontConfig(),
      loadFontOptions(),
    ])
    customFontInput.value = fontConfig.value.custom_font_family
  }
  catch (error) {
    console.error('加载字体设置失败:', error)
  }
})
</script>

<template>
  <!-- 设置内容 -->
  <n-space vertical size="large">
      <!-- 字体系列设置 -->
      <div>
        <div class="flex items-center justify-between mb-3">
          <div class="flex items-center">
            <div class="w-1.5 h-1.5 bg-orange-500 rounded-full mr-3 flex-shrink-0" />
            <div class="flex-1">
              <div class="text-sm font-medium leading-relaxed">
                字体系列
              </div>
              <div class="text-xs opacity-60">
                选择或自定义应用使用的字体系列
              </div>
            </div>
          </div>
          <!-- 重置按钮 -->
          <n-button
            size="small"
            type="default"
            :loading="isLoading"
            @click="handleResetConfig"
          >
            重置
          </n-button>
        </div>

        <!-- 字体选择器 -->
        <n-select
          :value="fontConfig.font_family"
          :options="fontFamilyOptions.map(opt => ({ label: opt.name, value: opt.id }))"
          :loading="isLoading"
          size="small"
          @update:value="handleFontFamilyChange"
        />

        <!-- 自定义字体输入（当选择自定义时显示） -->
        <n-collapse-transition :show="fontConfig.font_family === 'custom'">
          <div class="mt-3">
            <div class="text-xs opacity-60 mb-2">
              自定义字体CSS值
            </div>
            <n-input-group>
              <n-input
                v-model:value="customFontInput"
                placeholder="例如: 'MyFont', Arial, sans-serif"
                size="small"
                :disabled="isLoading"
                @keyup.enter="handleCustomFontFamilyChange"
              />
              <n-button
                type="primary"
                size="small"
                :loading="isLoading"
                @click="handleCustomFontFamilyChange"
              >
                应用
              </n-button>
            </n-input-group>
          </div>
        </n-collapse-transition>

        <div class="text-xs opacity-50 mt-2">
          当前: {{ currentFontFamilyName }}
        </div>
      </div>

      <!-- 快速应用字体 -->
      <div>
        <div class="flex items-center mb-3">
          <div class="w-1.5 h-1.5 bg-orange-500 rounded-full mr-3 flex-shrink-0" />
          <div class="flex-1">
            <div class="text-sm font-medium leading-relaxed">
              快速应用字体
            </div>
            <div class="text-xs opacity-60">
              输入系统中已安装的字体名称
            </div>
          </div>
        </div>

        <n-input-group>
          <n-input
            v-model:value="fontNameInput"
            placeholder="例如: Microsoft YaHei, 微软雅黑, PingFang SC"
            size="small"
            :disabled="isLoading"
            @keyup.enter="handleFontNameApply"
          />
          <n-button
            type="primary"
            size="small"
            :loading="isLoading"
            @click="handleFontNameApply"
          >
            应用
          </n-button>
        </n-input-group>

        <div class="text-xs opacity-50 mt-2">
          常用字体：微软雅黑、苹方、思源黑体、JetBrains Mono 等
        </div>
      </div>

      <!-- 字体大小设置 -->
      <div>
        <div class="flex items-center mb-3">
          <div class="w-1.5 h-1.5 bg-orange-500 rounded-full mr-3 flex-shrink-0" />
          <div class="flex-1">
            <div class="text-sm font-medium leading-relaxed">
              字体大小
            </div>
            <div class="text-xs opacity-60">
              调整应用界面的字体大小
            </div>
          </div>
        </div>
        <n-space>
          <n-button
            v-for="option in fontSizeOptions"
            :key="option.id"
            :type="fontConfig.font_size === option.id ? 'primary' : 'default'"
            size="small"
            :loading="isLoading"
            @click="handleFontSizeChange(option.id)"
          >
            {{ option.name }}
          </n-button>
        </n-space>
        <div class="text-xs opacity-50 mt-1">
          当前: {{ currentFontSizeName }} ({{ (currentFontScale * 100).toFixed(0) }}%)
        </div>
      </div>

      <!-- 字体预览 -->
      <div>
        <div class="flex items-center justify-between mb-3">
          <div class="flex items-center">
            <div class="w-1.5 h-1.5 bg-orange-500 rounded-full mr-3 flex-shrink-0" />
            <div>
              <div class="text-sm font-medium leading-relaxed">
                字体预览
              </div>
              <div class="text-xs opacity-60">
                实时预览当前字体效果
              </div>
            </div>
          </div>
        </div>
        <n-card
          size="small"
          :content-style="{
            fontFamily: currentFontFamily,
            fontSize: `${currentFontScale}rem`,
          }"
        >
          <div class="mb-3 font-medium text-lg">
            三术 - 智能代码审查工具
          </div>
          <div class="mb-3 opacity-80">
            The quick brown fox jumps over the lazy dog.
          </div>
          <div class="mb-3 opacity-80">
            微软雅黑 苹方 思源黑体 Source Han Sans
          </div>
          <div class="text-sm opacity-60 mb-2">
            ABCDEFGHIJKLMNOPQRSTUVWXYZ
          </div>
          <div class="text-sm opacity-60 mb-2">
            abcdefghijklmnopqrstuvwxyz
          </div>
          <div class="text-sm opacity-60">
            0123456789 !@#$%^&amp;*()_+-=[]{}|;:&apos;&quot;,./?
          </div>
        </n-card>
        <div class="text-xs opacity-50 mt-2 space-y-1">
          <div>当前字体: {{ currentFontFamily }}</div>
        </div>
      </div>
  </n-space>
</template>
