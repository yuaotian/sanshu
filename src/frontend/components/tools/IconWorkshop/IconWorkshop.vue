<script setup lang="ts">
/**
 * 图标工坊 - 主组件
 * 提供图标搜索、预览、复制和保存功能
 */
import { invoke } from '@tauri-apps/api/core'
import { useMessage } from 'naive-ui'
import { computed, onMounted, ref, watch } from 'vue'
import { useIconSearch } from '../../../composables/useIconSearch'
import { DEFAULT_FILTER_OPTIONS, type IconItem, type IconSaveRequest } from '../../../types/icon'
import IconResultsPanel from './IconResultsPanel.vue'
import IconSaveModal from './IconSaveModal.vue'

interface Props {
  active?: boolean
  // 弹窗模式相关参数
  mode?: 'normal' | 'popup'
  initialQuery?: string
  initialStyle?: string
  initialSavePath?: string
  projectRoot?: string
  // 外部保存模式（由父组件接管保存流程）
  externalSave?: boolean
}

const props = withDefaults(defineProps<Props>(), {
  active: false,
  mode: 'normal',
  initialQuery: '',
  initialStyle: 'all',
  initialSavePath: '',
  projectRoot: '',
  externalSave: false,
})

const emit = defineEmits<{
  // 外部保存请求（由父组件处理）
  save: [request: IconSaveRequest]
  // 选中图标变化通知（用于弹窗模式的编辑器）
  selectionChange: [icons: IconItem[]]
  // 双击图标打开编辑器
  iconDblClick: [icon: IconItem]
  // 右键图标打开上下文菜单
  iconContextMenu: [icon: IconItem, event: MouseEvent]
}>()

// 消息提示
const message = useMessage()

// 图标搜索 Hook
const {
  loading,
  error,
  searchParams,
  searchResult,
  icons,
  total,
  hasMore,
  currentPage,
  selectedIds,
  selectedIcons,
  selectedCount,
  hasSelection,
  isAllSelected,
  showFilters,
  showSaveModal,
  config,
  search,
  loadMore,
  toggleSelect,
  toggleSelectAll,
  clearSelection,
  copyToClipboard,
  saveIcons,
  loadConfig,
} = useIconSearch()

// 本地状态
const searchInput = ref('')

// 计算属性
const hasResults = computed(() => icons.value.length > 0)
const isEmpty = computed(() => !loading.value && !!searchInput.value.trim() && !hasResults.value)
const showEmptyState = computed(() => !loading.value && !searchInput.value && !hasResults.value)

// 默认展开筛选面板（主组件）
if (props.mode === 'normal')
  showFilters.value = true

// 通知父组件选中图标变化
watch(selectedIcons, (value) => {
  emit('selectionChange', value)
}, { immediate: true })

// 默认保存路径
const defaultSavePath = computed(() => {
  // 如果有初始保存路径（来自 CLI 参数），优先使用
  if (props.initialSavePath) {
    return props.initialSavePath
  }
  return config.value?.defaultSavePath || 'assets/icons'
})

// 执行搜索
async function handleSearch() {
  if (!searchInput.value.trim()) {
    message.warning('请输入搜索关键词')
    return
  }
  searchParams.query = searchInput.value.trim()
  await search()
}

// 筛选项变更时立即刷新结果
function handleFilterChange() {
  if (!searchParams.query.trim())
    return
  search()
}

// 回车搜索
function handleKeydown(e: KeyboardEvent) {
  if (e.key === 'Enter') {
    handleSearch()
  }
}

// 复制图标
async function handleCopy(icon: IconItem) {
  const success = await copyToClipboard(icon)
  if (success) {
    message.success(`已复制 ${icon.name} 到剪贴板`)
  }
  else {
    message.error('复制失败')
  }
}

// 双击图标 - 转发给父组件
function handleIconDblClick(icon: IconItem) {
  emit('iconDblClick', icon)
}

// 右键图标 - 转发给父组件
function handleIconContextMenu(icon: IconItem, event: MouseEvent) {
  emit('iconContextMenu', icon, event)
}

// 打开保存模态框
function openSaveModal() {
  if (!hasSelection.value) {
    message.warning('请先选择要保存的图标')
    return
  }
  showSaveModal.value = true
}

// 保存选中的图标
async function handleSave(request: IconSaveRequest) {
  // 外部保存模式：由父组件负责保存与后续流程
  if (props.externalSave) {
    showSaveModal.value = false
    emit('save', request)
    return
  }

  const result = await saveIcons(request)
  if (result) {
    message.success(`成功保存 ${result.successCount} 个图标`)
    showSaveModal.value = false
    clearSelection()

    // 如果是弹窗模式，保存成功后退出应用
    if (props.mode === 'popup') {
      try {
        // 构建响应数据
        const response = {
          saved_count: result.successCount,
          save_path: result.savePath,
          saved_names: result.items.filter((i: any) => i.success).map((i: any) => i.name),
          cancelled: false,
        }

        // 发送响应并退出
        await invoke('send_mcp_response', { response })
        await invoke('exit_app')
      }
      catch (e) {
        console.error('Failed to send response or exit:', e)
      }
    }
  }
}

// 加载更多
async function handleLoadMore() {
  if (loading.value || !hasMore.value)
    return
  await loadMore()
}

async function handleJumpPage(page: number) {
  if (loading.value)
    return
  if (!searchParams.query.trim())
    return
  searchResult.value = null
  searchParams.page = page
  clearSelection()
  await search(false)
}

// 组件挂载时加载配置
onMounted(async () => {
  await loadConfig()

  // 如果是弹窗模式，初始化参数并自动搜索
  if (props.mode === 'popup') {
    if (props.initialQuery) {
      searchInput.value = props.initialQuery
      // 这里的 searchParams 是 reactive 对象，直接赋值即可
      searchParams.query = props.initialQuery

      if (props.initialStyle && props.initialStyle !== 'all') {
        // 简单的类型检查，确保它是合法的样式值
        if (['line', 'fill', 'flat', 'all'].includes(props.initialStyle)) {
          searchParams.style = props.initialStyle as 'line' | 'fill' | 'flat' | 'all'
        }
      }

      // 自动执行搜索
      await search()
    }
  }
})
</script>

<template>
  <div class="relative flex flex-col gap-4" :class="mode === 'popup' ? 'h-full overflow-hidden' : ''">
    <!-- 搜索区域 -->
    <div class="flex flex-col gap-3 px-1 pt-1">
      <!-- 搜索栏 -->
      <div class="flex items-center gap-2">
        <n-input
          v-model:value="searchInput"
          placeholder="输入关键词搜索图标..."
          size="small"
          clearable
          class="flex-1"
          @keydown="handleKeydown"
        >
          <template #prefix>
            <div class="i-carbon-search text-lg text-on-surface-muted" />
          </template>
        </n-input>

        <n-button
          type="primary"
          size="small"
          :loading="loading"
          @click="handleSearch"
        >
          <template #icon>
            <div class="i-carbon-search" />
          </template>
          搜索
        </n-button>

        <n-button
          quaternary
          size="small"
          :type="showFilters ? 'primary' : 'default'"
          @click="showFilters = !showFilters"
        >
          <template #icon>
            <div class="i-carbon-filter" />
          </template>
          筛选
        </n-button>
      </div>

      <n-collapse-transition :show="showFilters">
        <n-card size="small">
          <n-space :size="[24, 8]" align="center" wrap>
            <n-space align="center" :size="8">
              <n-text depth="3" class="text-xs">
                风格
              </n-text>
              <n-radio-group v-model:value="searchParams.style" size="small" @update:value="handleFilterChange">
                <n-radio-button
                  v-for="opt in DEFAULT_FILTER_OPTIONS.styles"
                  :key="opt.value"
                  :value="opt.value"
                >
                  {{ opt.label }}
                </n-radio-button>
              </n-radio-group>
            </n-space>

            <n-space align="center" :size="8">
              <n-text depth="3" class="text-xs">
                填充
              </n-text>
              <n-radio-group v-model:value="searchParams.fills" size="small" @update:value="handleFilterChange">
                <n-radio-button
                  v-for="opt in DEFAULT_FILTER_OPTIONS.fills"
                  :key="opt.value"
                  :value="opt.value"
                >
                  {{ opt.label }}
                </n-radio-button>
              </n-radio-group>
            </n-space>

            <n-space align="center" :size="8">
              <n-text depth="3" class="text-xs">
                排序
              </n-text>
              <n-radio-group v-model:value="searchParams.sortType" size="small" @update:value="handleFilterChange">
                <n-radio-button
                  v-for="opt in DEFAULT_FILTER_OPTIONS.sortTypes"
                  :key="opt.value"
                  :value="opt.value"
                >
                  {{ opt.label }}
                </n-radio-button>
              </n-radio-group>
            </n-space>
          </n-space>
        </n-card>
      </n-collapse-transition>
    </div>

    <n-card v-if="hasResults || hasSelection" size="small" class="mx-1">
      <div class="flex justify-between items-center">
        <n-space align="center" :size="12">
          <n-checkbox
            :checked="isAllSelected"
            :indeterminate="hasSelection && !isAllSelected"
            @update:checked="toggleSelectAll"
          >
            全选
          </n-checkbox>
          <n-divider vertical />
          <n-text depth="3" class="text-xs">
            共 {{ total }} 个结果 · 第 {{ currentPage }} 页
          </n-text>
        </n-space>

        <n-space :size="8">
          <n-button
            v-if="hasSelection"
            size="small"
            quaternary
            type="error"
            @click="clearSelection"
          >
            清空 ({{ selectedCount }})
          </n-button>
          <n-button
            type="primary"
            size="small"
            :disabled="!hasSelection"
            @click="openSaveModal"
          >
            <template #icon>
              <div class="i-carbon-download" />
            </template>
            保存选中 ({{ selectedCount }})
          </n-button>
        </n-space>
      </div>
    </n-card>

    <IconResultsPanel
      :icons="icons"
      :selected-ids="selectedIds"
      :loading="loading"
      :has-more="hasMore"
      :current-page="currentPage"
      :page-size="searchParams.pageSize"
      :total="total"
      :is-empty="isEmpty"
      :show-empty-state="showEmptyState"
      :class="mode === 'normal' ? 'min-h-60' : ''"
      @toggle="toggleSelect"
      @copy="handleCopy"
      @dblclick="handleIconDblClick"
      @contextmenu="handleIconContextMenu"
      @load-more="handleLoadMore"
      @jump="handleJumpPage"
    />

    <!-- 错误提示 -->
    <div v-if="error" class="absolute bottom-20 left-1/2 transform -translate-x-1/2 z-50">
      <n-alert type="error" closable title="出错了">
        {{ error }}
      </n-alert>
    </div>

    <!-- 保存模态框 -->
    <IconSaveModal
      v-model:show="showSaveModal"
      :icons="selectedIcons"
      :default-path="defaultSavePath"
      @save="handleSave"
    />
  </div>
</template>

