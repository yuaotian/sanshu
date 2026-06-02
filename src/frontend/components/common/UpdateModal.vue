<script setup lang="ts">
import { useMessage } from 'naive-ui'
import { computed, onMounted, onUnmounted, ref } from 'vue'
import { useMarkdown } from '../../composables/useMarkdown'
import { useVersionCheck } from '../../composables/useVersionCheck'

const props = defineProps<Props>()

const emit = defineEmits<{
  'update:show': [value: boolean]
}>()

const { renderMarkdownSimple } = useMarkdown()

interface Props {
  show: boolean
  versionInfo: {
    current: string
    latest: string
    hasUpdate: boolean
    releaseUrl: string
    releaseNotes: string
  } | null
}

const message = useMessage()
const {
  isUpdating,
  updateStatus,
  updateProgress,
  networkStatus,
  platformInfo,
  autoExitCountdown,
  performOneClickUpdate,
  restartApp,
  exitForUpdate,
  getPlatformInfo,
  setupAutoExitListener,
  dismissUpdate,
} = useVersionCheck()

// 判断是否为 Windows 平台
const isWindows = computed(() => platformInfo.value === 'windows')

type StatusTagType = 'success' | 'warning' | 'error' | 'info'

interface StatusMeta {
  title: string
  description: string
  tagText: string
  tagType: StatusTagType
  icon: string
  panelClass: string
  progressColor: string
}

interface NetworkDetailRow {
  key: string
  label: string
  value: string
  icon: string
  title?: string
  mono?: boolean
  tagType?: StatusTagType
}

// 自动退出监听器清理函数
let unlistenAutoExit: (() => void) | null = null

// 组件挂载时初始化
onMounted(async () => {
  // 获取平台信息
  await getPlatformInfo()
  
  // 设置自动退出监听器（仅 Windows 平台需要）
  unlistenAutoExit = await setupAutoExitListener()
})

// 组件卸载时清理
onUnmounted(() => {
  if (unlistenAutoExit) {
    unlistenAutoExit()
    unlistenAutoExit = null
  }
})

// 网络状态面板展开状态
const showNetworkDetails = ref(false)

// 将更新状态集中映射为展示元信息，避免模板里堆叠复杂三元表达式
const updateStatusMeta = computed<StatusMeta>(() => {
  switch (updateStatus.value) {
    case 'checking':
      return {
        title: '正在校验更新',
        description: '正在确认版本与下载地址，请稍候。',
        tagText: '检查中',
        tagType: 'info',
        icon: 'i-carbon-search text-info',
        panelClass: 'border-info/40 bg-info/10',
        progressColor: '#3b82f6',
      }
    case 'downloading':
      return {
        title: '正在下载更新包',
        description: '下载期间请保持网络连接稳定。',
        tagText: '下载中',
        tagType: 'info',
        icon: 'i-carbon-download text-info',
        panelClass: 'border-info/40 bg-info/10',
        progressColor: '#3b82f6',
      }
    case 'installing':
      return {
        title: '正在安装更新',
        description: '更新包已下载，正在写入安装流程。',
        tagText: '安装中',
        tagType: 'warning',
        icon: 'i-carbon-renew text-warning',
        panelClass: 'border-warning/40 bg-warning/10',
        progressColor: '#f59e0b',
      }
    case 'completed':
      return {
        title: '更新已准备完成',
        description: isWindows.value ? '点击完成更新后应用会退出并完成替换。' : '请重启应用以使用最新版本。',
        tagText: '已完成',
        tagType: 'success',
        icon: 'i-carbon-checkmark-filled text-success',
        panelClass: 'border-success/40 bg-success/10',
        progressColor: '#22c55e',
      }
    case 'error':
      return {
        title: '更新未完成',
        description: '请检查网络状态后重试，必要时可手动下载最新版本。',
        tagText: '异常',
        tagType: 'error',
        icon: 'i-carbon-warning-alt text-error',
        panelClass: 'border-error/40 bg-error/10',
        progressColor: '#ef4444',
      }
    default:
      return {
        title: '准备更新',
        description: '确认后将自动下载并安装最新版本。',
        tagText: '待开始',
        tagType: 'info',
        icon: 'i-carbon-upgrade text-primary-500',
        panelClass: 'border-primary-500/30 bg-primary-500/10',
        progressColor: '#14b8a6',
      }
  }
})

const showStatusPanel = computed(() =>
  isUpdating.value || updateStatus.value === 'completed' || updateStatus.value === 'error',
)

// 获取国家名称（简单映射）
function getCountryName(code: string): string {
  const countryMap: Record<string, string> = {
    CN: '中国',
    US: '美国',
    JP: '日本',
    KR: '韩国',
    HK: '香港',
    TW: '台湾',
    SG: '新加坡',
    DE: '德国',
    GB: '英国',
    FR: '法国',
    UNKNOWN: '未知',
  }
  return countryMap[code] || code
}

// 获取连接方式描述
const connectionDescription = computed(() => {
  if (!networkStatus.value)
    return '检测中...'
  if (networkStatus.value.using_proxy) {
    const proxyType = networkStatus.value.proxy_type?.toUpperCase() || 'HTTP'
    const proxyHost = networkStatus.value.proxy_host || '代理节点'
    const proxyPort = networkStatus.value.proxy_port ? `:${networkStatus.value.proxy_port}` : ''
    return `代理 (${proxyType} ${proxyHost}${proxyPort})`
  }
  return '直连'
})

const networkTagType = computed<StatusTagType>(() => {
  if (!networkStatus.value)
    return 'info'
  return networkStatus.value.github_reachable ? 'success' : 'warning'
})

const networkTagText = computed(() => {
  if (!networkStatus.value)
    return '检测中'
  return networkStatus.value.github_reachable ? '正常' : '受限'
})

const networkSummary = computed(() => {
  if (!networkStatus.value)
    return '正在检测网络环境'
  const location = `${getCountryName(networkStatus.value.country)} (${networkStatus.value.country})`
  const github = networkStatus.value.github_reachable ? 'GitHub 可达' : 'GitHub 受限'
  return `${connectionDescription.value} / ${location} / ${github}`
})

const networkDetailRows = computed<NetworkDetailRow[]>(() => {
  const status = networkStatus.value
  const location = status ? `${getCountryName(status.country)} (${status.country})` : '检测中...'
  const rows: NetworkDetailRow[] = [
    {
      key: 'location',
      label: '当前位置',
      value: location,
      icon: 'i-carbon-location text-info',
      title: location,
    },
    {
      key: 'connection',
      label: '连接方式',
      value: connectionDescription.value,
      icon: status?.using_proxy ? 'i-carbon-connection-signal text-primary-500' : 'i-carbon-direct-link text-primary-500',
      title: connectionDescription.value,
    },
    {
      key: 'github',
      label: 'GitHub 连接',
      value: status ? (status.github_reachable ? '正常' : '不可达') : '检测中',
      icon: 'i-carbon-logo-github text-on-surface-secondary',
      tagType: status ? (status.github_reachable ? 'success' : 'error') : 'info',
    },
  ]

  if (status?.ip && status.ip !== 'unknown') {
    rows.push({
      key: 'ip',
      label: '出口 IP',
      value: status.ip,
      icon: 'i-carbon-ip text-info',
      title: status.ip,
      mono: true,
    })
  }

  return rows
})

function formatBytes(value?: number): string {
  if (!value)
    return '0 KB'
  const kb = value / 1024
  if (kb < 1024)
    return `${Math.round(kb * 100) / 100} KB`
  return `${Math.round(kb / 1024 * 100) / 100} MB`
}

// 使用共享 markdown 实例渲染更新说明
const formattedReleaseNotes = computed(() => {
  if (!props.versionInfo?.releaseNotes)
    return ''
  return renderMarkdownSimple(props.versionInfo.releaseNotes)
})

const isVisible = computed({
  get: () => props.show,
  set: value => emit('update:show', value),
})

// 确认更新
async function handleConfirmUpdate() {
  try {
    message.info('正在准备更新...')
    await performOneClickUpdate()

    if (updateStatus.value === 'completed') {
      message.success('更新完成！')
    }
  }
  catch (error) {
    const errorMsg = error instanceof Error ? error.message : String(error)
    console.error('更新失败:', errorMsg)

    // 如果是需要手动下载的错误，引导用户手动下载
    if (errorMsg.includes('手动下载') || errorMsg.includes('网络请求受限') || errorMsg.includes('403')) {
      let warningMsg = '自动更新不可用，将为您打开下载页面'

      if (errorMsg.includes('网络请求受限') || errorMsg.includes('403')) {
        warningMsg = '网络请求受限，将为您打开下载页面'
      }

      message.warning(warningMsg)

      // 打开下载页面
      if (props.versionInfo?.releaseUrl) {
        try {
          window.open(props.versionInfo.releaseUrl, '_blank')
        }
        catch (openError) {
          console.error('打开下载页面失败:', openError)
          message.error('无法打开下载页面，请手动访问 GitHub 下载最新版本')
        }
      }
      else {
        message.error('无法获取下载链接，请手动访问 GitHub 下载最新版本')
      }

      // 延迟关闭弹窗，让用户看到提示
      setTimeout(() => {
        isVisible.value = false
      }, 2000)
    }
    else {
      // 其他错误显示具体错误信息
      let displayMsg = errorMsg || '更新失败，请稍后重试'

      // 检查是否是网络相关错误
      if (errorMsg.includes('网络') || errorMsg.includes('连接') || errorMsg.includes('请求失败')
        || errorMsg.includes('timeout') || errorMsg.includes('ENOTFOUND') || errorMsg.includes('ECONNREFUSED')) {
        displayMsg = '网络连接异常，请检查网络后重试'
      }

      message.error(`更新失败: ${displayMsg}`)
    }
  }
}

// 关闭弹窗（不再提醒）
function handleDismiss() {
  dismissUpdate()
  message.info('已关闭更新提醒')
}

// 重启应用（非 Windows 平台使用）
async function handleRestart() {
  try {
    await restartApp()
  }
  catch (error) {
    console.error('重启失败:', error)
    message.error('重启失败，请手动重启应用')
  }
}

// 手动触发退出更新（Windows 平台使用，当用户点击按钮时）
async function handleExitForUpdate() {
  try {
    message.info('正在完成更新，应用即将退出...')
    await exitForUpdate()
  }
  catch (error) {
    console.error('退出失败:', error)
    message.error('退出失败，请手动关闭应用完成更新')
  }
}
</script>

<template>
  <n-modal
    v-model:show="isVisible"
    :mask-closable="false"
    :close-on-esc="false"
    preset="dialog"
    class="update-modal w-[92vw] max-w-[94vw] sm:w-[560px] md:w-[640px]"
    :style="{ maxHeight: '86vh' }"
  >
    <template #header>
      <div class="flex items-start gap-3">
        <div class="mt-0.5 flex h-9 w-9 shrink-0 items-center justify-center rounded-lg border border-primary-500/30 bg-primary-500/10">
          <div class="i-carbon-upgrade h-5 w-5 text-primary-500" />
        </div>
        <div class="min-w-0">
          <div class="text-base font-semibold leading-6 text-on-surface">
            发现新版本
          </div>
          <div v-if="versionInfo" class="mt-0.5 truncate text-xs text-on-surface-secondary">
            已检测到 v{{ versionInfo.latest }}，当前运行 v{{ versionInfo.current }}
          </div>
        </div>
      </div>
    </template>

    <div class="max-h-[72vh] overflow-hidden">
      <!-- 版本信息 -->
      <div v-if="versionInfo" class="space-y-4">
        <div class="rounded-lg border border-primary-500/30 bg-primary-500/10 p-4">
          <div class="version-compare-grid grid items-center gap-3">
            <div class="min-w-0 rounded-lg border border-surface-200 bg-surface-50 p-3 dark:border-surface-700 dark:bg-surface-100">
              <div class="mb-1 text-xs text-on-surface-secondary">
                当前版本
              </div>
              <div class="truncate font-mono text-sm font-semibold text-on-surface" :title="`v${versionInfo.current}`">
                v{{ versionInfo.current }}
              </div>
            </div>

            <div class="flex h-8 w-8 shrink-0 items-center justify-center rounded-full border border-primary-500/30 bg-primary-500/10">
              <div class="i-carbon-arrow-right h-4 w-4 text-primary-500" />
            </div>

            <div class="min-w-0 rounded-lg border border-primary-500/40 bg-primary-500/10 p-3">
              <div class="mb-1 text-xs text-on-surface-secondary">
                最新版本
              </div>
              <div class="truncate font-mono text-sm font-semibold text-primary-500" :title="`v${versionInfo.latest}`">
                v{{ versionInfo.latest }}
              </div>
            </div>
          </div>
        </div>

        <!-- 网络状态（可折叠） -->
        <div class="overflow-hidden rounded-lg border border-surface-200 bg-surface-50 dark:border-surface-700 dark:bg-surface-100">
          <!-- 折叠头部 -->
          <div
            class="flex cursor-pointer items-center justify-between gap-3 p-3 transition-colors hover:bg-surface-100 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-primary-500/30 dark:hover:bg-surface-200"
            role="button"
            tabindex="0"
            :aria-expanded="showNetworkDetails"
            aria-label="展开或收起网络状态详情"
            @click="showNetworkDetails = !showNetworkDetails"
            @keydown.enter.space.prevent="showNetworkDetails = !showNetworkDetails"
          >
            <div class="flex min-w-0 items-center gap-3">
              <div class="flex h-8 w-8 shrink-0 items-center justify-center rounded-lg bg-success/10">
                <div class="i-carbon-network-3 h-4 w-4 text-success" />
              </div>
              <div class="min-w-0">
                <div class="flex items-center gap-2">
                  <span class="text-sm font-medium text-on-surface">网络状态</span>
                  <!-- 简要状态指示 -->
                  <n-tag
                    size="tiny"
                    :type="networkTagType"
                  >
                    {{ networkTagText }}
                  </n-tag>
                </div>
                <div class="mt-0.5 truncate text-xs text-on-surface-secondary" :title="networkSummary">
                  {{ networkSummary }}
                </div>
              </div>
            </div>
            <div
              class="i-carbon-chevron-down h-4 w-4 shrink-0 text-on-surface-secondary transition-transform duration-200"
              :class="{ 'rotate-180': showNetworkDetails }"
            />
          </div>

          <!-- 折叠内容 -->
          <n-collapse-transition :show="showNetworkDetails">
            <div class="grid gap-2 border-t border-surface-200 p-3 dark:border-surface-700 sm:grid-cols-2">
              <div
                v-for="row in networkDetailRows"
                :key="row.key"
                class="min-w-0 rounded-lg border border-surface-200 bg-surface p-3 dark:border-surface-700"
              >
                <div class="mb-1 flex items-center gap-2 text-xs text-on-surface-secondary">
                  <div :class="[row.icon, 'h-3.5 w-3.5 shrink-0']" />
                  <span>{{ row.label }}</span>
                </div>
                <n-tag
                  v-if="row.tagType"
                  size="tiny"
                  :type="row.tagType"
                >
                  {{ row.value }}
                </n-tag>
                <div
                  v-else
                  class="truncate text-sm font-medium text-on-surface"
                  :class="{ 'font-mono text-xs': row.mono }"
                  :title="row.title || row.value"
                >
                  {{ row.value }}
                </div>
              </div>
            </div>
          </n-collapse-transition>
        </div>

        <!-- 更新状态与进度 -->
        <div
          v-if="showStatusPanel"
          class="rounded-lg border p-4 transition-colors"
          :class="updateStatusMeta.panelClass"
        >
          <div class="space-y-3">
            <div class="flex items-start gap-3">
              <div class="mt-0.5 flex h-8 w-8 shrink-0 items-center justify-center rounded-lg bg-surface/80">
                <n-spin v-if="isUpdating && updateStatus !== 'completed' && updateStatus !== 'error'" size="small" />
                <div v-else :class="[updateStatusMeta.icon, 'h-4 w-4']" />
              </div>
              <div class="min-w-0 flex-1">
                <div class="flex flex-wrap items-center gap-2">
                  <span class="text-sm font-semibold text-on-surface">{{ updateStatusMeta.title }}</span>
                  <n-tag size="tiny" :type="updateStatusMeta.tagType">
                    {{ updateStatusMeta.tagText }}
                  </n-tag>
                </div>
                <p class="mt-1 text-xs leading-5 text-on-surface-secondary">
                  {{ updateStatusMeta.description }}
                </p>
              </div>
            </div>

            <!-- 下载进度条 -->
            <div v-if="updateProgress && updateStatus === 'downloading'" class="space-y-2">
              <n-progress
                type="line"
                :percentage="Math.round(updateProgress.percentage)"
                :show-indicator="false"
                :height="8"
                :color="updateStatusMeta.progressColor"
              />
              <div class="flex flex-wrap justify-between gap-2 text-xs text-on-surface-secondary">
                <span>{{ formatBytes(updateProgress.downloaded) }}</span>
                <span v-if="updateProgress.content_length">
                  / {{ formatBytes(updateProgress.content_length) }}
                </span>
                <span>{{ Math.round(updateProgress.percentage) }}%</span>
              </div>
            </div>
          </div>
        </div>

        <!-- 更新说明 -->
        <div v-if="versionInfo.releaseNotes && !isUpdating" class="space-y-3">
          <div class="flex items-center justify-between gap-3">
            <div class="flex items-center gap-2">
              <div class="i-carbon-document h-4 w-4 text-primary-500" />
              <h4 class="text-sm font-semibold text-on-surface">
                更新内容
              </h4>
            </div>
            <n-tag size="tiny" type="info">
              Release Notes
            </n-tag>
          </div>
          <n-scrollbar class="release-notes-scroll max-h-[260px] rounded-lg border border-surface-200 bg-surface-50 dark:border-surface-700 dark:bg-surface-100">
            <div class="p-4 text-sm text-on-surface-secondary">
              <div
                class="release-notes-content space-y-2"
                v-html="formattedReleaseNotes"
              />
            </div>
          </n-scrollbar>
        </div>
      </div>
    </div>

    <template #action>
      <div class="flex w-full flex-col-reverse gap-2 sm:flex-row sm:justify-end sm:gap-3">
        <!-- 关闭按钮 -->
        <n-button
          v-if="updateStatus !== 'completed'"
          class="w-full sm:w-auto"
          :disabled="isUpdating"
          @click="handleDismiss"
        >
          关闭
        </n-button>

        <!-- 立即更新按钮 -->
        <n-button
          v-if="updateStatus !== 'completed'"
          class="w-full sm:w-auto"
          type="primary"
          :loading="isUpdating"
          @click="handleConfirmUpdate"
        >
          <template #icon>
            <div class="i-carbon-upgrade" />
          </template>
          立即更新
        </n-button>

        <!-- Windows 平台：自动退出倒计时按钮 -->
        <n-button
          v-if="updateStatus === 'completed' && isWindows"
          class="w-full sm:w-auto"
          type="success"
          :loading="autoExitCountdown > 0"
          @click="handleExitForUpdate"
        >
          <template #icon>
            <div class="i-carbon-power" />
          </template>
          {{ autoExitCountdown > 0 ? `即将退出 (${autoExitCountdown}s)` : '完成更新' }}
        </n-button>

        <!-- 非 Windows 平台：重启按钮 -->
        <n-button
          v-if="updateStatus === 'completed' && !isWindows"
          class="w-full sm:w-auto"
          type="success"
          @click="handleRestart"
        >
          <template #icon>
            <div class="i-carbon-restart" />
          </template>
          重启应用
        </n-button>
      </div>
    </template>
  </n-modal>
</template>

<style scoped>
.update-modal :deep(.n-dialog__content) {
  padding-top: 4px;
}

.release-notes-scroll :deep(.n-scrollbar-content) {
  min-width: 0;
}

.version-compare-grid {
  grid-template-columns: minmax(0, 1fr) auto minmax(0, 1fr);
}

.release-notes-content {
  overflow-wrap: anywhere;
}

.release-notes-content :deep(:first-child) {
  margin-top: 0;
}

.release-notes-content :deep(:last-child) {
  margin-bottom: 0;
}

.release-notes-content :deep(h1),
.release-notes-content :deep(h2),
.release-notes-content :deep(h3),
.release-notes-content :deep(h4) {
  font-weight: 600;
  margin: 0.75rem 0 0.5rem 0;
  color: var(--text-color-1);
}

.release-notes-content :deep(h2) {
  font-size: 1.05em;
  border-bottom: 1px solid var(--border-color);
  padding-bottom: 0.25rem;
}

.release-notes-content :deep(h3) {
  font-size: 1em;
}

.release-notes-content :deep(p) {
  margin: 0.5rem 0;
  line-height: 1.6;
}

.release-notes-content :deep(ul),
.release-notes-content :deep(ol) {
  margin: 0.5rem 0;
  padding-left: 1.25rem;
}

.release-notes-content :deep(li) {
  margin: 0.25rem 0;
  line-height: 1.55;
}

.release-notes-content :deep(strong) {
  font-weight: 600;
  color: var(--text-color-1);
}

.release-notes-content :deep(em) {
  font-style: italic;
}

.release-notes-content :deep(code) {
  padding: 0.125rem 0.375rem;
  font-size: 0.875em;
  border-radius: 0.25rem;
  font-family: ui-monospace, SFMono-Regular, 'SF Mono', monospace;
  background-color: var(--code-color);
  color: var(--text-color-1);
  border: 1px solid var(--border-color);
}

.release-notes-content :deep(blockquote) {
  margin: 0.75rem 0;
  padding: 0.5rem 1rem;
  border-left: 3px solid var(--primary-color);
  background-color: var(--code-color);
  border-radius: 0 0.25rem 0.25rem 0;
}

/* 代码块使用横向滚动，避免长路径撑破弹窗 */
.release-notes-content :deep(pre) {
  margin: 0.75rem 0;
  padding: 0.75rem 1rem;
  border-radius: 0.375rem;
  overflow-x: auto;
  font-family: ui-monospace, SFMono-Regular, 'SF Mono', monospace;
  font-size: 0.8125em;
  line-height: 1.5;
  background-color: var(--code-color);
  border: 1px solid var(--border-color);
}

.release-notes-content :deep(pre code) {
  padding: 0;
  background-color: transparent;
  border: none;
  font-size: inherit;
}

/* 链接保持主色，和项目主题变量一致 */
.release-notes-content :deep(a) {
  color: var(--primary-color);
  text-decoration: none;
  transition: opacity 0.2s;
}

.release-notes-content :deep(a:hover) {
  opacity: 0.8;
  text-decoration: underline;
}

/* 分隔线样式 */
.release-notes-content :deep(hr) {
  margin: 1rem 0;
  border: none;
  border-top: 1px solid var(--border-color);
}

/* 表格样式（如果有） */
.release-notes-content :deep(table) {
  width: 100%;
  border-collapse: collapse;
  margin: 0.75rem 0;
  font-size: 0.875em;
}

.release-notes-content :deep(th),
.release-notes-content :deep(td) {
  padding: 0.5rem;
  border: 1px solid var(--border-color);
  text-align: left;
}

.release-notes-content :deep(th) {
  background-color: var(--code-color);
  font-weight: 600;
}
</style>
