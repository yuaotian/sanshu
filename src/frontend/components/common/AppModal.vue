<script setup lang="ts">
import { computed, useSlots } from 'vue';

defineOptions({ inheritAttrs: false })

const props = withDefaults(defineProps<{
  show: boolean
  preset?: 'card' | 'dialog'
  title?: string
  width?: string
  maxHeight?: string
  headerOffset?: number
  bodyOverflow?: 'auto' | 'hidden'
  bordered?: boolean
  size?: 'small' | 'medium' | 'huge'
  maskClosable?: boolean
  closeOnEsc?: boolean
  closable?: boolean
  segmented?: object | boolean
  // dialog preset props
  type?: 'default' | 'info' | 'success' | 'warning' | 'error'
  content?: string
  positiveText?: string
  negativeText?: string
  positiveButtonProps?: object
  negativeButtonProps?: object
}>(), {
  preset: 'card',
  title: '',
  maxHeight: '85vh',
  headerOffset: undefined,
  bodyOverflow: 'auto',
  bordered: false,
  size: 'small',
  maskClosable: true,
  closeOnEsc: true,
  closable: true,
  segmented: undefined,
  type: undefined,
  content: undefined,
  positiveText: undefined,
  negativeText: undefined,
  positiveButtonProps: undefined,
  negativeButtonProps: undefined,
})

const emit = defineEmits<{
  'update:show': [value: boolean]
  'after-leave': []
  'positive-click': []
  'negative-click': []
}>()

defineSlots<{
  default(): any
  header(): any
  footer(): any
  action(): any
}>()

const slots = useSlots()

const hasCustomHeader = computed(() => !!slots.header)
const useCustomCardHeader = computed(() => props.preset === 'card' && props.closable && !hasCustomHeader.value)

const computedOffset = computed(() => {
  if (props.headerOffset !== undefined) return props.headerOffset
  if (props.preset === 'dialog') return 80
  return slots.footer ? 120 : 100
})

const modalStyle = computed(() => {
  if (props.preset === 'dialog') {
    return {
      ...(props.width != null && props.width !== '' ? { width: props.width } : {}),
      maxHeight: props.maxHeight,
    }
  }
  return {
    ...(props.width != null && props.width !== '' ? { width: props.width } : {}),
    maxWidth: '95vw',
    maxHeight: props.maxHeight,
  }
})

const bodyStyle = computed(() => ({
  maxHeight: `calc(${props.maxHeight} - ${computedOffset.value}px)`,
  overflow: props.bodyOverflow,
}))

function handleClose() {
  emit('update:show', false)
}
</script>

<template>
  <n-modal
    v-bind="$attrs"
    :show="show"
    :preset="preset"
    :title="useCustomCardHeader ? undefined : title"
    :style="modalStyle"
    :body-style="bodyStyle"
    :bordered="bordered"
    :size="size"
    :mask-closable="maskClosable"
    :close-on-esc="closeOnEsc"
    :closable="useCustomCardHeader ? false : closable"
    :segmented="segmented"
    :type="type"
    :content="content"
    :positive-text="positiveText"
    :negative-text="negativeText"
    :positive-button-props="positiveButtonProps"
    :negative-button-props="negativeButtonProps"
    transform-origin="center"
    @update:show="emit('update:show', $event)"
    @after-leave="emit('after-leave')"
    @positive-click="emit('positive-click')"
    @negative-click="emit('negative-click')"
  >
    <slot />

    <template v-if="hasCustomHeader" #header>
      <slot name="header" />
    </template>
    <template v-else-if="useCustomCardHeader" #header>
      <div class="flex items-center justify-between w-full">
        <span class="text-base font-medium">{{ title }}</span>
        <n-button quaternary circle size="tiny" @click="handleClose">
          <template #icon>
            <div class="i-carbon-close text-base" />
          </template>
        </n-button>
      </div>
    </template>

    <template v-if="slots.footer" #footer>
      <slot name="footer" />
    </template>
    <template v-if="slots.action" #action>
      <slot name="action" />
    </template>
  </n-modal>
</template>
