import { invoke } from '@tauri-apps/api/core'
import { useMagicKeys } from '@vueuse/core'
import { computed, ref, watch } from 'vue'
import type { ShortcutBinding, ShortcutConfig, ShortcutKey } from '../types/popup'

/**
 * 自定义快捷键管理
 */
export function useShortcuts() {
  const shortcutConfig = ref<ShortcutConfig>({
    shortcuts: {},
  })

  const keys = useMagicKeys()

  // 检测操作系统
  const isMac = computed(() => {
    if (typeof navigator !== 'undefined') {
      return navigator.platform.toUpperCase().includes('MAC')
    }
    return false
  })

  // 加载快捷键配置
  async function loadShortcutConfig() {
    try {
      const config = await invoke<ShortcutConfig>('get_shortcut_config')
      shortcutConfig.value = config
    }
    catch (error) {
      console.error('加载快捷键配置失败:', error)
    }
  }

  // 保存快捷键配置
  async function saveShortcutBinding(shortcutId: string, binding: ShortcutBinding) {
    try {
      await invoke('update_shortcut_binding', {
        shortcutId,
        binding,
      })
      shortcutConfig.value.shortcuts[shortcutId] = binding
    }
    catch (error) {
      console.error('保存快捷键配置失败:', error)
      throw error
    }
  }

  // 重置快捷键为默认值
  async function resetShortcutsToDefault() {
    try {
      await invoke('reset_shortcuts_to_default')
      await loadShortcutConfig()
    }
    catch (error) {
      console.error('重置快捷键失败:', error)
      throw error
    }
  }

  // 将快捷键组合转换为字符串表示
  function shortcutKeyToString(shortcutKey: ShortcutKey): string {
    const parts: string[] = []

    if (isMac.value) {
      if (shortcutKey.meta)
        parts.push('⌘')
      if (shortcutKey.ctrl)
        parts.push('⌃')
      if (shortcutKey.alt)
        parts.push('⌥')
      if (shortcutKey.shift)
        parts.push('⇧')
    }
    else {
      if (shortcutKey.ctrl)
        parts.push('Ctrl')
      if (shortcutKey.alt)
        parts.push('Alt')
      if (shortcutKey.shift)
        parts.push('Shift')
      if (shortcutKey.meta)
        parts.push('Meta')
    }

    parts.push(shortcutKey.key)
    return parts.join(isMac.value ? '' : '+')
  }

  // 将快捷键组合转换为useMagicKeys格式
  function shortcutKeyToMagicKey(shortcutKey: ShortcutKey): string {
    const parts: string[] = []

    if (shortcutKey.ctrl)
      parts.push('Ctrl')
    if (shortcutKey.alt)
      parts.push('Alt')
    if (shortcutKey.shift)
      parts.push('Shift')
    if (shortcutKey.meta)
      parts.push('Meta')

    parts.push(shortcutKey.key)
    return parts.join('+')
  }

  // 检查快捷键是否冲突（全局唯一，不区分作用域）
  function checkShortcutConflict(newBinding: ShortcutBinding, excludeId?: string): string | null {
    const newKeyStr = shortcutKeyToMagicKey(newBinding.key_combination)

    for (const [id, binding] of Object.entries(shortcutConfig.value.shortcuts)) {
      if (id === excludeId)
        continue

      const existingKeyStr = shortcutKeyToMagicKey(binding.key_combination)
      if (existingKeyStr === newKeyStr) {
        return binding.name
      }
    }

    return null
  }

  // 获取指定动作的快捷键
  function getShortcutByAction(action: string): ShortcutBinding | null {
    for (const binding of Object.values(shortcutConfig.value.shortcuts)) {
      if (binding.action === action) {
        return binding
      }
    }
    return null
  }

  // 获取快速发送快捷键的显示文本
  const quickSubmitShortcutText = computed(() => {
    const binding = getShortcutByAction('submit')
    if (!binding) {
      return isMac.value ? '⌘+回车 快速发送' : 'Ctrl+回车 快速发送'
    }
    return `${shortcutKeyToString(binding.key_combination)} ${binding.name}`
  })

  // 获取增强快捷键的显示文本
  const enhanceShortcutText = computed(() => {
    const binding = getShortcutByAction('enhance')
    if (!binding) {
      return isMac.value ? '⌘+⇧+回车 增强' : 'Ctrl+Shift+回车 增强'
    }
    return `${shortcutKeyToString(binding.key_combination)} ${binding.name}`
  })

  // 获取继续快捷键的显示文本
  const continueShortcutText = computed(() => {
    const binding = getShortcutByAction('continue')
    if (!binding) {
      return isMac.value ? '⌥+回车 继续' : 'Alt+回车 继续'
    }
    return `${shortcutKeyToString(binding.key_combination)} ${binding.name}`
  })

  function useShortcutWatch(action: string, callback: () => void) {
    const binding = computed(() => getShortcutByAction(action))

    watch(
      () => binding.value,
      (newBinding, _old, onCleanup) => {
        if (!newBinding?.enabled)
          return

        const magicKey = shortcutKeyToMagicKey(newBinding.key_combination)
        const keyRef = keys[magicKey]

        if (keyRef) {
          const stop = watch(keyRef, (pressed) => {
            if (pressed) {
              callback()
            }
          })
          onCleanup(stop)
        }
      },
      { immediate: true },
    )
  }

  function useQuickSubmitShortcut(callback: () => void) {
    useShortcutWatch('submit', callback)
  }

  function useEnhanceShortcut(callback: () => void) {
    useShortcutWatch('enhance', callback)
  }

  function useContinueShortcut(callback: () => void) {
    useShortcutWatch('continue', callback)
  }

  return {
    shortcutConfig,
    isMac,
    loadShortcutConfig,
    saveShortcutBinding,
    resetShortcutsToDefault,
    shortcutKeyToString,
    shortcutKeyToMagicKey,
    checkShortcutConflict,
    getShortcutByAction,
    quickSubmitShortcutText,
    enhanceShortcutText,
    continueShortcutText,
    useQuickSubmitShortcut,
    useEnhanceShortcut,
    useContinueShortcut,
  }
}
