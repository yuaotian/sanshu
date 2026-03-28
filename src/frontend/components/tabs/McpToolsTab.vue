<script setup lang="ts">
import { useMessage } from 'naive-ui'
import { computed, defineAsyncComponent, onMounted, ref, watch } from 'vue'
import { useMcpToolsReactive } from '../../composables/useMcpTools'
import AppModal from '../common/AppModal.vue'

// 异步加载配置组件
const SouConfig = defineAsyncComponent(() => import('../tools/SouConfig.vue'))
const Context7Config = defineAsyncComponent(() => import('../tools/Context7Config.vue'))
const IconWorkshop = defineAsyncComponent(() => import('../tools/IconWorkshop/IconWorkshop.vue'))
const EnhanceConfig = defineAsyncComponent(() => import('../tools/EnhanceConfig.vue'))
const MemoryConfig = defineAsyncComponent(() => import('../tools/MemoryConfig.vue'))

const props = withDefaults(defineProps<{
  projectRootPath?: string | null
  autoOpenToolId?: string | null
  autoOpenToolRequestId?: number
}>(), {
  autoOpenToolId: null,
  autoOpenToolRequestId: 0,
})

const emit = defineEmits<{
  autoOpenHandled: [requestId: number]
}>()

// 全局 MCP 工具状态
const {
  mcpTools,
  loading,
  loadMcpTools,
  toggleTool: globalToggleTool,
  toolStats,
} = useMcpToolsReactive()

const message = useMessage()
const needsReconnect = ref(false)
const showToolConfigModal = ref(false)
const currentToolId = ref('')
const lastHandledAutoOpenRequestId = ref(0)

// 计算属性：当前工具名称
const currentToolName = computed(() => {
  const tool = mcpTools.value.find(t => t.id === currentToolId.value)
  return tool ? tool.name : ''
})

// 切换工具启用状态
async function toggleTool(toolId: string) {
  try {
    const result = await globalToggleTool(toolId)
    if (result.needsReconnect) {
      needsReconnect.value = true
    }
    message.warning('MCP工具配置已更新，请在MCP客户端中重连服务')
  }
  catch (err) {
    message.error(`更新MCP工具状态失败: ${err}`)
  }
}

// 打开工具配置弹窗
function openToolConfig(toolId: string) {
  currentToolId.value = toolId
  showToolConfigModal.value = true
}

watch(
  () => [props.autoOpenToolId, props.autoOpenToolRequestId] as const,
  ([toolId, requestId]) => {
    if (!toolId || !requestId || requestId === lastHandledAutoOpenRequestId.value)
      return

    // 通过 requestId 去重，避免组件重挂载时重复弹出 sou 配置。
    lastHandledAutoOpenRequestId.value = requestId
    openToolConfig(toolId)
    emit('autoOpenHandled', requestId)
  },
  { immediate: true },
)

// 组件挂载时加载工具列表
onMounted(async () => {
  try {
    await loadMcpTools()
  }
  catch (err) {
    message.error(`加载MCP工具配置失败: ${err}`)
  }
})
</script>

<template>
  <div class="tab-content">
    <!-- 重连提示 -->
    <n-alert
      v-if="needsReconnect"
      title="需要重连MCP服务"
      type="warning"
      closable
      class="mb-4"
      @close="needsReconnect = false"
    >
      <template #icon>
        <div class="i-carbon-connection-signal text-lg" />
      </template>
      MCP工具配置已更改，请在MCP客户端中重连三术服务。
    </n-alert>

    <!-- 工具卡片网格 -->
    <n-grid
      :cols="4"
      :x-gap="16"
      :y-gap="16"
      item-responsive
      responsive="screen"
    >
      <!-- 骨架屏 -->
      <template v-if="loading">
        <n-grid-item v-for="n in 6" :key="'skeleton-' + n" span="4 s:2 m:2 l:1">
          <n-card size="small" class="h-full">
            <template #header>
              <n-space align="center">
                <n-skeleton height="40px" width="40px" :sharp="false" />
                <div>
                  <n-skeleton text width="100px" class="mb-1" />
                  <n-skeleton text width="140px" />
                </div>
              </n-space>
            </template>
            <n-skeleton text :repeat="2" />
          </n-card>
        </n-grid-item>
      </template>

      <!-- 工具卡片 -->
      <template v-else>
        <n-grid-item
          v-for="tool in mcpTools"
          :key="tool.id"
          span="4 s:2 m:2 l:1"
        >
          <n-card
            size="small"
            class="h-full transition-all duration-300 hover:shadow-md hover:-translate-y-0.5"
            :class="{ 'opacity-50': !tool.enabled }"
          >
            <template #header>
              <n-space align="center">
                <div
                  class="w-10 h-10 rounded-xl flex items-center justify-center"
                  :class="[tool.icon_bg, tool.dark_icon_bg]"
                >
                  <div class="text-xl" :class="[tool.icon]" />
                </div>
                <div>
                  <div class="flex items-center gap-1.5">
                    <span class="text-base font-semibold leading-tight">{{ tool.name }}</span>
                    <n-tag
                      v-if="!tool.can_disable"
                      type="info"
                      size="tiny"
                      round
                      :bordered="false"
                    >
                      核心
                    </n-tag>
                  </div>
                  <div class="text-xs text-on-surface-secondary mt-0.5">
                    {{ tool.description }}
                  </div>
                </div>
              </n-space>
            </template>

            <template #header-extra>
              <n-switch
                v-if="tool.can_disable"
                :value="tool.enabled"
                size="small"
                @update:value="toggleTool(tool.id)"
              />
            </template>

            <!-- 操作区域 -->
            <div class="flex items-center justify-between">
              <n-button
                v-if="tool.has_config"
                size="small"
                secondary
                @click="openToolConfig(tool.id)"
              >
                <template #icon>
                  <div class="i-carbon-settings" />
                </template>
                配置
              </n-button>
              <span v-else class="text-xs opacity-40">暂无配置项</span>

              <n-tag
                :type="tool.enabled ? 'success' : 'default'"
                size="tiny"
                round
                :bordered="false"
              >
                {{ tool.enabled ? '运行中' : '已禁用' }}
              </n-tag>
            </div>
          </n-card>
        </n-grid-item>
      </template>
    </n-grid>

    <!-- 底部统计 -->
    <div class="flex justify-center py-4">
      <n-tag round :bordered="false" size="small">
        <template #icon>
          <div class="i-carbon-tool-kit" />
        </template>
        {{ toolStats.enabled }} / {{ toolStats.total }} 工具正在运行
      </n-tag>
    </div>

    <!-- 配置弹窗 -->
    <AppModal
      v-model:show="showToolConfigModal"
      :title="`${currentToolName} 配置`"
      width="760px"
    >
      <div>
        <SouConfig v-if="currentToolId === 'sou'" :active="showToolConfigModal" />
        <Context7Config v-else-if="currentToolId === 'context7'" :active="showToolConfigModal" />
        <EnhanceConfig
          v-else-if="currentToolId === 'enhance'"
          :active="showToolConfigModal"
          :project-root-path="props.projectRootPath"
        />
        <IconWorkshop v-else-if="currentToolId === 'icon'" :active="showToolConfigModal" />
        <MemoryConfig
          v-else-if="currentToolId === 'ji'"
          :active="showToolConfigModal"
          :project-root-path="props.projectRootPath"
        />
        <n-empty
          v-else
          size="small"
          description="暂无高级配置项"
          class="min-h-64 flex justify-center items-center"
        >
          <template #icon>
            <div class="i-carbon-settings text-on-surface-muted" />
          </template>
        </n-empty>
      </div>
    </AppModal>
  </div>
</template>
