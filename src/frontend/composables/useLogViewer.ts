import { ref } from 'vue'

// 中文说明：全局单例的日志查看器开关状态
// 目的：让“主界面 / 弹窗 Header / 工具配置页”等任意位置都能打开同一个日志抽屉
let instance: {
  show: ReturnType<typeof ref<boolean>>
  open: () => void
  close: () => void
} | null = null

export function useLogViewer() {
  if (!instance) {
    const show = ref(false)
    instance = {
      show,
      open: () => { show.value = true },
      close: () => { show.value = false },
    }
  }
  return instance
}

