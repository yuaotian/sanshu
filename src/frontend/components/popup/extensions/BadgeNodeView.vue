<script setup lang="ts">
import { invoke } from '@tauri-apps/api/core';
import { NodeViewWrapper } from '@tiptap/vue-3';
import { useMessage } from 'naive-ui';
import { computed, onBeforeUnmount, onMounted, ref } from 'vue';

const props = defineProps<{
  node: {
    attrs: {
      badgeType: 'url' | 'path'
      identity: string
      label: string
      kind: string
      serialized: string
      referenceData: string
      title: string
    }
    nodeSize: number
  }
  editor: { state: { doc: { nodeAt: (pos: number) => { isText: boolean, text?: string } | null } }, chain: () => any }
  getPos: () => number
  deleteNode: () => void
}>()

const message = useMessage()
const showMenu = ref(false)
const badgeRef = ref<HTMLElement | null>(null)

const isUrl = computed(() => props.node.attrs.badgeType === 'url')

const kindIcon = computed(() => {
  const { badgeType, kind } = props.node.attrs
  if (badgeType === 'url') return 'i-carbon-link'
  if (kind === '目录') return 'i-carbon-folder'
  return 'i-carbon-document'
})

function handleBadgeClick(event: Event) {
  if (!isUrl.value) return
  event.preventDefault()
  event.stopPropagation()
  showMenu.value = !showMenu.value
}

function handleOpen() {
  showMenu.value = false
  const data = props.node.attrs.referenceData
  try {
    const ref = JSON.parse(data)
    if (ref.url) {
      invoke('open_external_url', { url: ref.url }).catch((e: unknown) => {
        console.error('打开失败:', e)
        message.error(`无法打开: ${ref.url}`)
      })
    }
  }
  catch {}
}

function handleUnlink() {
  showMenu.value = false
  const pos = props.getPos()
  if (typeof pos !== 'number') return

  const label = props.node.attrs.title || props.node.attrs.label
  props.editor.chain().command(({ tr }: any) => {
    const nodeEnd = pos + props.node.nodeSize
    tr.replaceWith(pos, nodeEnd, (tr.doc.type.schema as any).text(label))
    return true
  }).run()
}

function deleteBadgeWithSpace(event: Event) {
  event.preventDefault()
  event.stopPropagation()
  showMenu.value = false

  const pos = props.getPos()
  if (typeof pos !== 'number') {
    props.deleteNode()
    return
  }

  const { state } = props.editor
  const nodeEnd = pos + props.node.nodeSize
  const afterNode = state.doc.nodeAt(nodeEnd)
  const deleteEnd = (afterNode?.isText && /^[\s\u00a0]/.test(afterNode.text || ''))
    ? nodeEnd + 1
    : nodeEnd

  props.editor.chain().command(({ tr }: any) => {
    tr.delete(pos, deleteEnd)
    return true
  }).run()
}

function onClickOutside(e: MouseEvent) {
  if (badgeRef.value && !badgeRef.value.contains(e.target as Node)) {
    showMenu.value = false
  }
}

onMounted(() => document.addEventListener('click', onClickOutside, true))
onBeforeUnmount(() => document.removeEventListener('click', onClickOutside, true))
</script>

<template>
  <NodeViewWrapper
    ref="badgeRef"
    as="span"
    class="popup-inline-reference"
    :title="node.attrs.title"
  >
    <span class="popup-inline-reference-icon-slot">
      <span class="popup-inline-reference-kind" :class="kindIcon" />
      <span class="popup-inline-reference-delete" @click="deleteBadgeWithSpace">
        <span class="i-carbon-close w-2.5 h-2.5" />
      </span>
    </span>
    <span
      class="popup-inline-reference-label"
      :class="{ 'cursor-pointer': isUrl }"
      @click="handleBadgeClick"
    >{{ node.attrs.label }}</span>

    <span v-if="showMenu && isUrl" class="badge-popover">
      <span class="badge-popover-item" @click="handleOpen">
        <span class="i-carbon-launch w-3 h-3" />
        <span>Open</span>
      </span>
      <span class="badge-popover-divider" />
      <span class="badge-popover-item" @click="handleUnlink">
        <span class="i-carbon-unlink w-3 h-3" />
        <span>Unlink</span>
      </span>
    </span>
  </NodeViewWrapper>
</template>
