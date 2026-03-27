import { invoke } from '@tauri-apps/api/core'
import { emit, listen } from '@tauri-apps/api/event'
import { computed, ref } from 'vue'
import { applyThemeVariables, getTheme } from '../theme'

const currentTheme = ref('')
let initialized = false

function applyTheme(theme: string) {
  applyThemeVariables(theme)
  currentTheme.value = theme
}

async function setTheme(theme: string) {
  try {
    await invoke('set_theme', { theme })
    applyTheme(theme)
    emit('sanshu://theme-changed', { theme })
  }
  catch (error) {
    console.error('保存主题设置失败:', error)
  }
}

async function loadTheme() {
  try {
    const theme = await invoke('get_theme')
    const validTheme = (theme === 'light' || theme === 'dark') ? theme : 'dark'
    applyTheme(validTheme as string)
  }
  catch (error) {
    console.error('加载主题失败:', error)
    applyTheme('dark')
  }
}

export function useTheme() {
  if (!initialized) {
    initialized = true

    listen<{ theme: string }>('sanshu://theme-changed', (event) => {
      if (event.payload.theme && event.payload.theme !== currentTheme.value) {
        applyTheme(event.payload.theme)
      }
    })

    loadTheme().catch(() => {
      applyTheme('dark')
    })
  }

  const naiveTheme = computed(() => {
    const theme = currentTheme.value || 'dark'
    return getTheme(theme)
  })

  return {
    currentTheme,
    naiveTheme,
    setTheme,
    loadTheme,
  }
}
