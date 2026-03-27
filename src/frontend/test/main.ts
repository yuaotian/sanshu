// 导入 Naive UI 组件 (复用主应用的配置)
import {
    create,
    NButton,
    NCard,
    NCheckbox,
    NConfigProvider,
    NDialogProvider,
    NGrid,
    NGridItem,
    NImage,
    NImageGroup,
    NInput,
    NInputGroup,
    NInputNumber,
    NMessageProvider,
    NNotificationProvider,
    NSpace,
    NSwitch,
    NTab,
    NTabPane,
    NTabs,
    NTag,
} from 'naive-ui'
import { createApp } from 'vue'

// 导入主题
import { useTheme } from '../composables/useTheme'

import TestApp from './TestApp.vue'
// 导入样式
import 'virtual:uno.css'

import '../assets/styles/style.css'

// 创建 Naive UI 实例
const naive = create({
  components: [
    NButton,
    NCard,
    NCheckbox,
    NConfigProvider,
    NGrid,
    NGridItem,
    NImage,
    NImageGroup,
    NInput,
    NInputGroup,
    NInputNumber,
    NSpace,
    NSwitch,
    NTab,
    NTabPane,
    NTabs,
    NTag,
    NMessageProvider,
    NNotificationProvider,
    NDialogProvider,
  ],
})

// 创建 Vue 应用
const app = createApp(TestApp)

// 使用 Naive UI
app.use(naive)

// 挂载应用
app.mount('#app')

// 模拟 Tauri API 用于测试环境
if (!(window as any).__TAURI__) {
  (window as any).__TAURI__ = {
    core: {
      invoke: async (command: string) => {
        console.log(`模拟 Tauri 调用: ${command}`)
        // 模拟主题配置返回
        if (command === 'get_theme_config') {
          return { theme: 'dark' }
        }
        if (command === 'get_reply_config') {
          return { continue_prompt: '请按照最佳实践继续' }
        }
        return {}
      },
    },
  }
}

// 初始化主题
const { loadTheme } = useTheme()
loadTheme().catch(() => {
  console.log('使用默认主题')
})
