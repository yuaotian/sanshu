<script setup lang="ts">
import { computed } from 'vue'

interface HistoryEntry {
  id: string
  user_input: string
  ai_response_summary: string
  timestamp: string
}

interface Props {
  includeContext: boolean
  includeHistory: boolean
  useDefaultRule: boolean
  customRuleInput: string
  customRuleMax: number
  historyEntries: HistoryEntry[]
  selectedHistoryIds: string[]
  historyLoading: boolean
  historyError: string
  defaultRuleText: string
  isMobile: boolean
}

interface Emits {
  'update:includeContext': [value: boolean]
  'update:includeHistory': [value: boolean]
  'update:useDefaultRule': [value: boolean]
  'update:customRuleInput': [value: string]
  'update:selectedHistoryIds': [value: string[]]
}

const props = defineProps<Props>()
const emit = defineEmits<Emits>()

const selectedIds = computed({
  get: () => props.selectedHistoryIds,
  set: value => emit('update:selectedHistoryIds', value),
})

const customRuleCount = computed(() => props.customRuleInput.length)

function handleSelectAll() {
  emit('update:selectedHistoryIds', props.historyEntries.map(entry => entry.id))
}

function handleClearAll() {
  emit('update:selectedHistoryIds', [])
}

// 截断历史摘要文本，保持列表紧凑
function truncateText(text: string, maxLength = 50) {
  if (!text) return ''
  return text.length > maxLength ? `${text.slice(0, maxLength)}...` : text
}

// 格式化时间戳，便于快速浏览
function formatTimestamp(timestamp: string) {
  if (!timestamp) return ''
  const date = new Date(timestamp)
  if (Number.isNaN(date.getTime())) return timestamp
  return date.toLocaleString()
}
</script>

<template>
  <n-card size="small" bordered class="!rounded-[3px] shadow-sm border-border bg-container">
    <template #header>
      <div class="flex items-center gap-2 text-sm font-medium text-on-surface-secondary">
        <div class="i-carbon-settings-services h-4 w-4 text-on-surface-muted" />
        可选配置
      </div>
    </template>

    <n-collapse :accordion="isMobile" class="space-y-2">
      <n-collapse-item name="context">
        <template #header>
          <div class="flex items-center gap-2 text-sm text-on-surface-secondary">
            <div class="i-carbon-settings-adjust h-4 w-4 text-on-surface-muted" />
            上下文选项
          </div>
        </template>
        <div class="space-y-3">
          <n-checkbox
            :checked="includeContext"
            @update:checked="(value: boolean) => emit('update:includeContext', value)"
          >
            添加快捷上下文
          </n-checkbox>

          <n-card v-if="includeContext" embedded size="small" class="!rounded-[3px]">
            <slot name="context-preview" />
          </n-card>
          <div v-else class="text-xs text-on-surface-muted">
            未启用上下文补充
          </div>
        </div>
      </n-collapse-item>

      <n-collapse-item name="history">
        <template #header>
          <div class="flex items-center gap-2 text-sm text-on-surface-secondary">
            <div class="i-carbon-chat h-4 w-4 text-on-surface-muted" />
            历史记录
          </div>
        </template>
        <div class="space-y-3">
          <n-checkbox
            :checked="includeHistory"
            @update:checked="(value: boolean) => emit('update:includeHistory', value)"
          >
            包含对话历史
          </n-checkbox>

          <div v-if="includeHistory" class="space-y-2">
            <div class="flex items-center justify-between text-xs text-on-surface-muted">
              <span>最近 5 条增强记录</span>
              <div class="flex items-center gap-2">
                <n-button size="tiny" secondary @click="handleSelectAll">全选</n-button>
                <n-button size="tiny" secondary @click="handleClearAll">全不选</n-button>
              </div>
            </div>

            <div v-if="historyLoading" class="space-y-2">
              <n-skeleton height="20px" width="90%" class="animate-pulse" />
              <n-skeleton height="20px" width="80%" class="animate-pulse" />
              <n-skeleton height="20px" width="85%" class="animate-pulse" />
            </div>

            <div v-else-if="historyError" class="text-xs text-error">
              {{ historyError }}
            </div>

            <div v-else-if="historyEntries.length === 0" class="text-xs text-on-surface-muted">
              暂无可用历史记录
            </div>

            <n-checkbox-group v-else v-model:value="selectedIds" class="space-y-2">
              <n-card
                v-for="entry in historyEntries"
                :key="entry.id"
                size="small"
                class="!rounded-[3px] text-xs text-on-surface-secondary"
              >
                <div class="flex items-start gap-2">
                  <n-checkbox :value="entry.id" />
                  <div class="flex-1 space-y-1">
                    <div class="text-[11px] text-on-surface-muted">
                      {{ formatTimestamp(entry.timestamp) }}
                    </div>
                    <div>
                      <span class="text-on-surface-muted">原始：</span>{{ truncateText(entry.user_input) }}
                    </div>
                    <div>
                      <span class="text-on-surface-muted">增强：</span>{{ truncateText(entry.ai_response_summary) }}
                    </div>
                  </div>
                </div>
              </n-card>
            </n-checkbox-group>
          </div>
          <div v-else class="text-xs text-on-surface-muted">
            已关闭对话历史
          </div>
        </div>
      </n-collapse-item>

      <n-collapse-item name="rules">
        <template #header>
          <div class="flex items-center gap-2 text-sm text-on-surface-secondary">
            <div class="i-carbon-rule h-4 w-4 text-on-surface-muted" />
            增强规则
          </div>
        </template>
        <div class="space-y-3">
          <n-checkbox
            :checked="useDefaultRule"
            @update:checked="(value: boolean) => emit('update:useDefaultRule', value)"
          >
            使用默认规则（中文优先）
          </n-checkbox>
          <n-card embedded size="small" class="!rounded-[3px] text-xs text-on-surface-secondary">
            {{ defaultRuleText }}
          </n-card>

          <div class="space-y-2">
            <div class="flex items-center justify-between text-xs text-on-surface-muted">
              <span>自定义规则</span>
              <span>{{ customRuleCount }} / {{ customRuleMax }}</span>
            </div>
            <n-input
              type="textarea"
              size="small"
              :value="customRuleInput"
              :maxlength="customRuleMax"
              :autosize="{ minRows: 3, maxRows: 4 }"
              placeholder="例如：强调代码安全性、优先使用函数式编程风格等"
              @update:value="(value: string) => emit('update:customRuleInput', value)"
            />
          </div>
        </div>
      </n-collapse-item>
    </n-collapse>
  </n-card>
</template>
