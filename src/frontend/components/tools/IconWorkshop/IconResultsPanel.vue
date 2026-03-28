<script setup lang="ts">
/**
 * 图标结果面板
 * 负责瀑布流渲染、加载更多与滚动触发
 */
import { computed, onBeforeUnmount, ref, watch } from 'vue'
import type { IconItem } from '../../../types/icon'
import IconCard from './IconCard.vue'
import IconCardSkeleton from './IconCardSkeleton.vue'

interface Props {
  icons: IconItem[]
  selectedIds: Set<number>
  loading: boolean
  hasMore: boolean
  currentPage: number
  pageSize: number
  total: number
  isEmpty: boolean
  showEmptyState: boolean
}

const props = defineProps<Props>()

const emit = defineEmits<{
  toggle: [iconId: number]
  copy: [icon: IconItem]
  dblclick: [icon: IconItem]
  contextmenu: [icon: IconItem, event: MouseEvent]
  'load-more': []
  jump: [page: number]
}>()

const hasResults = computed(() => props.icons.length > 0)
const maxPage = computed(() => Math.max(1, Math.ceil(props.total / props.pageSize)))
const scrollContainer = ref<HTMLElement | null>(null)
const loadMoreTrigger = ref<HTMLElement | null>(null)
const isAutoLoading = ref(false)
const jumpPage = ref<number | null>(null)
let observer: IntersectionObserver | null = null

function handleLoadMore() {
  if (props.loading || !props.hasMore)
    return
  emit('load-more')
}

function handleJump() {
  if (!jumpPage.value)
    return
  const target = Math.min(Math.max(1, Math.floor(jumpPage.value)), maxPage.value)
  emit('jump', target)
}

watch(() => props.loading, (value) => {
  if (!value)
    isAutoLoading.value = false
})

watch(
  [loadMoreTrigger, scrollContainer, () => props.hasMore],
  ([trigger, container, hasMore]) => {
    if (observer) {
      observer.disconnect()
      observer = null
    }
    if (!trigger || !container || !hasMore)
      return

    // 底部进入视口时自动加载，增强瀑布流体验
    observer = new IntersectionObserver((entries) => {
      if (!entries.some(entry => entry.isIntersecting))
        return
      if (isAutoLoading.value || props.loading || !props.hasMore)
        return
      isAutoLoading.value = true
      emit('load-more')
    }, {
      root: container,
      rootMargin: '0px 0px 240px 0px',
      threshold: 0.1,
    })
    observer.observe(trigger)
  },
  { immediate: true },
)

onBeforeUnmount(() => {
  if (observer)
    observer.disconnect()
})
</script>

<template>
  <div ref="scrollContainer" class="flex-1 overflow-y-auto min-h-0 pr-2 relative">
    <!-- 初始骨架屏 -->
    <div v-if="loading && !hasResults" class="columns-4 sm:columns-5 md:columns-6 lg:columns-8 gap-3">
      <div v-for="i in 32" :key="`skeleton-${i}`" class="mb-3 break-inside-avoid">
        <IconCardSkeleton />
      </div>
    </div>

    <template v-else-if="hasResults">
      <transition-group
        tag="div"
        class="columns-4 sm:columns-5 md:columns-6 lg:columns-8 gap-3"
        enter-active-class="transition-all duration-300 ease-out"
        enter-from-class="opacity-0 translate-y-2"
        enter-to-class="opacity-100 translate-y-0"
        leave-active-class="transition-all duration-200 ease-in"
        leave-from-class="opacity-100"
        leave-to-class="opacity-0"
      >
        <div
          v-for="icon in icons"
          :key="icon.id"
          class="mb-3 break-inside-avoid"
        >
          <IconCard
            :icon="icon"
            :selected="selectedIds.has(icon.id)"
            @toggle="emit('toggle', icon.id)"
            @copy="emit('copy', icon)"
            @dblclick="emit('dblclick', icon)"
            @contextmenu="emit('contextmenu', icon, $event)"
          />
        </div>
      </transition-group>

      <!-- 自动加载哨兵 -->
      <div v-if="hasMore" ref="loadMoreTrigger" class="h-1 w-full" />

      <div v-if="loading && hasMore" class="flex items-center justify-center py-4 text-xs text-on-surface-muted">
        <n-space align="center" :size="8">
          <n-spin size="small" />
          <span>加载中...</span>
        </n-space>
      </div>

      <!-- 手动加载按钮 -->
      <div v-if="hasMore && !loading" class="flex justify-center py-6">
        <n-button secondary size="small" @click="handleLoadMore">
          加载更多
        </n-button>
      </div>
    </template>

    <n-empty
      v-else-if="isEmpty"
      description="未找到相关图标，请尝试其他关键词"
      class="h-full flex flex-col justify-center items-center text-on-surface-muted"
    >
      <template #icon>
        <div class="i-carbon-search-locate text-4xl opacity-50" />
      </template>
    </n-empty>

    <n-empty
      v-else-if="showEmptyState"
      class="h-full flex flex-col justify-center items-center text-on-surface-disabled"
    >
      <template #icon>
        <div class="i-carbon-image text-8xl opacity-10" />
      </template>
      <template #default>
        <div class="text-center">
          <p class="text-lg font-medium opacity-80 mb-2">搜索 Iconfont 图标库</p>
          <p class="text-sm opacity-50">输入关键词开始探索无限创意</p>
        </div>
      </template>
    </n-empty>

    <!-- 悬浮分页组件（在滚动容器内，使用 sticky 定位） -->
    <div
      v-if="hasResults"
      class="sticky bottom-2 mx-auto z-40 flex items-center gap-3 px-4 py-2 rounded-full bg-surface border border-border shadow-xl backdrop-blur-sm w-fit"
    >
      <span class="text-xs text-on-surface-secondary whitespace-nowrap">
        第 {{ currentPage }} 页
      </span>
      <span class="text-xs text-on-surface-muted whitespace-nowrap">
        / {{ maxPage }} 页
      </span>
      <div class="h-4 w-px bg-border" />
      <span class="text-xs text-on-surface-secondary whitespace-nowrap">
        共 {{ total }} 个
      </span>
      <div class="h-4 w-px bg-border" />
      <div class="flex items-center gap-2">
        <n-input-number
          v-model:value="jumpPage"
          size="tiny"
          :min="1"
          :max="maxPage"
          :disabled="loading"
          class="w-20"
          placeholder="页码"
        />
        <n-button size="tiny" secondary :disabled="loading" @click="handleJump">
          跳转
        </n-button>
      </div>
      <n-button
        v-if="hasMore"
        size="tiny"
        type="primary"
        :loading="loading"
        @click="handleLoadMore"
      >
        加载更多
      </n-button>
      <span v-else class="text-xs text-on-surface-muted">已全部加载</span>
    </div>
  </div>
</template>

