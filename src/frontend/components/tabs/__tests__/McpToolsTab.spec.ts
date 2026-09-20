import { flushPromises, mount } from '@vue/test-utils'
import { describe, expect, it, vi } from 'vitest'
import { defineComponent } from 'vue'
import McpToolsTab from '../McpToolsTab.vue'

vi.mock('naive-ui', () => ({ useMessage: () => ({ warning: vi.fn(), error: vi.fn() }) }))
vi.mock('../../../composables/useMcpTools', async () => {
  const { ref } = await import('vue')
  return {
    useMcpToolsReactive: () => ({
      mcpTools: ref([]),
      loading: ref(false),
      loadMcpTools: vi.fn(),
      toggleTool: vi.fn(),
      toolStats: ref({ enabled: 0, total: 0 }),
    }),
  }
})
vi.mock('../../tools/UiuxConfig.vue', async () => {
  const { defineComponent } = await import('vue')
  return {
    __esModule: true,
    default: defineComponent({
      props: ['session', 'active'],
      emits: ['update:session'],
      data() {
        return {
          snapshot: {
            saved: { knowledge_backend: 'auto', semantic_enabled: true, model_dir: 'A', effective_model_dir: 'A' },
            draft: { knowledge_backend: 'local', semantic_enabled: true, model_dir: 'B', effective_model_dir: 'A' },
          },
        }
      },
      template: '<section class="uiux-editor"><span>{{ session?.draft.model_dir || "无草稿" }}</span><button @click="$emit(\'update:session\', snapshot)">编辑草稿</button></section>',
    }),
  }
})
vi.mock('../../tools/SouConfig.vue', () => ({ __esModule: true, default: { template: '<section class="sou-editor">Sou</section>' } }))

describe('mcp 工具配置会话', () => {
  it('工具切换卸载 UIUX 后，父局部缓存仍将草稿交回新组件', async () => {
    const slots = defineComponent({ template: '<div><slot /></div>' })
    const wrapper = mount(McpToolsTab, {
      props: { autoOpenToolId: 'uiux', autoOpenToolRequestId: 1 },
      global: { stubs: { 'n-space': slots, 'n-modal': slots, 'n-tag': slots, 'n-alert': slots, 'n-button': slots, 'n-switch': true } },
    })
    await flushPromises()
    await wrapper.get('.uiux-editor button').trigger('click')
    await wrapper.setProps({ autoOpenToolId: 'sou', autoOpenToolRequestId: 2 })
    await flushPromises()
    expect(wrapper.find('.uiux-editor').exists()).toBe(false)
    expect(wrapper.find('.sou-editor').exists()).toBe(true)
    await wrapper.setProps({ autoOpenToolId: 'uiux', autoOpenToolRequestId: 3 })
    await flushPromises()
    expect(wrapper.get('.uiux-editor').text()).toContain('B')
    wrapper.unmount()
  })
})
