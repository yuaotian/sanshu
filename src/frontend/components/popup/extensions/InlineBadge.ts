import { mergeAttributes, Node } from '@tiptap/core'
import { Plugin, PluginKey } from '@tiptap/pm/state'
import { NodeSelection, TextSelection } from '@tiptap/pm/state'
import { Decoration, DecorationSet } from '@tiptap/pm/view'
import { VueNodeViewRenderer } from '@tiptap/vue-3'
import BadgeNodeView from './BadgeNodeView.vue'

const badgeSelectionKey = new PluginKey('inlineBadgeSelection')

export interface InlineBadgeAttrs {
  badgeType: 'url' | 'path'
  identity: string
  label: string
  kind: string
  serialized: string
  referenceData: string
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
    return VueNodeViewRenderer(BadgeNodeView as any, {
      stopEvent: () => true,
    })
  },

  addProseMirrorPlugins() {
    const typeName = this.name
    return [
      new Plugin({
        props: {
          handleKeyDown(view, event) {
            if (event.key !== 'ArrowLeft' && event.key !== 'ArrowRight') return false

            const { state } = view
            const { selection } = state
            const isLeft = event.key === 'ArrowLeft'

            if (selection instanceof NodeSelection && selection.node.type.name === typeName) {
              if (event.shiftKey) {
                const anchor = isLeft ? selection.to : selection.from
                const head = isLeft ? selection.from : selection.to
                view.dispatch(state.tr.setSelection(TextSelection.create(state.doc, anchor, head)))
              }
              else {
                const pos = isLeft ? selection.from : selection.to
                view.dispatch(state.tr.setSelection(TextSelection.create(state.doc, pos)))
              }
              return true
            }

            if (event.shiftKey) return false

            if (selection instanceof TextSelection && selection.empty) {
              const { $from } = selection
              if (isLeft) {
                const before = $from.nodeBefore
                if (before?.type.name === typeName) {
                  view.dispatch(state.tr.setSelection(TextSelection.create(state.doc, $from.pos - before.nodeSize)))
                  return true
                }
              }
              else {
                const after = $from.nodeAfter
                if (after?.type.name === typeName) {
                  view.dispatch(state.tr.setSelection(TextSelection.create(state.doc, $from.pos + after.nodeSize)))
                  return true
                }
              }
            }

            return false
          },
        },
      }),
      new Plugin({
        key: badgeSelectionKey,
        state: {
          init() { return DecorationSet.empty },
          apply(tr, _oldDecos, _oldState, newState) {
            const { selection } = newState
            if (selection.empty && !(selection instanceof NodeSelection))
              return DecorationSet.empty

            const decorations: Decoration[] = []
            const { from, to } = selection

            newState.doc.nodesBetween(from, to, (node, pos) => {
              if (node.type.name === typeName && pos >= from && pos + node.nodeSize <= to) {
                decorations.push(Decoration.node(pos, pos + node.nodeSize, { class: 'badge-in-selection' }))
              }
            })

            return decorations.length ? DecorationSet.create(newState.doc, decorations) : DecorationSet.empty
          },
        },
        props: {
          decorations(state) { return this.getState(state) },
        },
      }),
    ]
  },
})
