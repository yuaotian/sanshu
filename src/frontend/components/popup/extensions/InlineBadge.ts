import { mergeAttributes, Node } from '@tiptap/core'
import { VueNodeViewRenderer } from '@tiptap/vue-3'
import BadgeNodeView from './BadgeNodeView.vue'

export interface InlineBadgeAttrs {
  badgeType: 'url' | 'path' | 'image'
  identity: string
  label: string
  kind: string
  serialized: string
  referenceData: string
  imageBadgeId: string | null
  title: string
}

export const InlineBadge = Node.create({
  name: 'inlineBadge',
  group: 'inline',
  inline: true,
  atom: true,
  selectable: true,
  draggable: false,

  addAttributes() {
    return {
      badgeType: { default: 'path' },
      identity: { default: '' },
      label: { default: '' },
      kind: { default: '' },
      serialized: { default: '' },
      referenceData: { default: '' },
      imageBadgeId: { default: null },
      title: { default: '' },
    }
  },

  parseHTML() {
    return [{ tag: 'span[data-inline-badge]' }]
  },

  renderHTML({ HTMLAttributes }) {
    return ['span', mergeAttributes(HTMLAttributes, { 'data-inline-badge': '' })]
  },

  addNodeView() {
    return VueNodeViewRenderer(BadgeNodeView as any)
  },
})
