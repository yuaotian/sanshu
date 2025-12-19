import { computed, onUnmounted, ref } from 'vue'

/**
 * 相对时间格式化 Composable
 * 提供相对时间显示和自动更新功能
 */
export function useRelativeTime() {
  // 当前时间戳，用于触发响应式更新
  const now = ref(Date.now())
  let timer: number | null = null

  /**
   * 格式化为相对时间
   * @param timeStr ISO 时间字符串或 null
   * @returns 相对时间文本，如 "刚刚"、"5 分钟前"
   */
  function formatRelative(timeStr: string | null): string {
    if (!timeStr) return '从未'

    try {
      const date = new Date(timeStr)
      const diffMs = now.value - date.getTime()
      const diffSec = Math.floor(diffMs / 1000)
      const diffMin = Math.floor(diffSec / 60)
      const diffHour = Math.floor(diffMin / 60)
      const diffDay = Math.floor(diffHour / 24)
      const diffMonth = Math.floor(diffDay / 30)
      const diffYear = Math.floor(diffDay / 365)

      if (diffSec < 0) return '刚刚' // 处理时间偏差
      if (diffSec < 60) return '刚刚'
      if (diffMin < 60) return `${diffMin} 分钟前`
      if (diffHour < 24) return `${diffHour} 小时前`
      if (diffDay < 30) return `${diffDay} 天前`
      if (diffMonth < 12) return `${diffMonth} 个月前`
      return `${diffYear} 年前`
    } catch {
      return '未知'
    }
  }

  /**
   * 格式化为绝对时间
   * @param timeStr ISO 时间字符串或 null
   * @returns 绝对时间文本，如 "2024/01/15 14:30:00"
   */
  function formatAbsolute(timeStr: string | null): string {
    if (!timeStr) return '从未索引'

    try {
      return new Date(timeStr).toLocaleString('zh-CN', {
        year: 'numeric',
        month: '2-digit',
        day: '2-digit',
        hour: '2-digit',
        minute: '2-digit',
        second: '2-digit',
      })
    } catch {
      return '时间格式错误'
    }
  }

  /**
   * 创建响应式相对时间
   * @param timeStr ISO 时间字符串或 null
   * @returns 响应式的相对时间 computed
   */
  function createRelativeTime(timeStr: () => string | null) {
    return computed(() => formatRelative(timeStr()))
  }

  /**
   * 开始自动更新时间（每分钟更新一次）
   */
  function startAutoUpdate(intervalMs = 60000) {
    if (timer) return

    timer = window.setInterval(() => {
      now.value = Date.now()
    }, intervalMs)
  }

  /**
   * 停止自动更新
   */
  function stopAutoUpdate() {
    if (timer) {
      clearInterval(timer)
      timer = null
    }
  }

  // 组件卸载时清理定时器
  onUnmounted(() => {
    stopAutoUpdate()
  })

  return {
    now,
    formatRelative,
    formatAbsolute,
    createRelativeTime,
    startAutoUpdate,
    stopAutoUpdate,
  }
}

