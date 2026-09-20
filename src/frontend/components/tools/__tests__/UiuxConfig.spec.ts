import type { VueWrapper } from '@vue/test-utils'
import type { UiuxConfigData, UiuxEditSession } from '../../../types/uiux'
import { flushPromises, mount } from '@vue/test-utils'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { defineComponent } from 'vue'
import UiuxConfig from '../UiuxConfig.vue'

const mocks = vi.hoisted(() => ({
  invoke: vi.fn(),
  success: vi.fn(),
  error: vi.fn(),
  info: vi.fn(),
  warning: vi.fn(),
}))
vi.mock('@tauri-apps/api/core', () => ({ invoke: mocks.invoke }))
vi.mock('naive-ui', () => ({
  useMessage: () => ({ success: mocks.success, error: mocks.error, info: mocks.info }),
  useDialog: () => ({ warning: mocks.warning }),
}))

function deferred<T>() {
  let resolve!: (value: T) => void
  let reject!: (error: unknown) => void
  const promise = new Promise<T>((resolvePromise, rejectPromise) => {
    resolve = resolvePromise
    reject = rejectPromise
  })
  return { promise, resolve, reject }
}

const slots = defineComponent({ template: '<div><slot /><slot name="feedback" /><slot name="trigger" /></div>' })
const button = defineComponent({
  props: ['disabled', 'loading'],
  emits: ['click'],
  template: '<button :disabled="disabled" @click="$emit(\'click\')"><slot /></button>',
})
const input = defineComponent({
  props: ['value', 'disabled'],
  emits: ['update:value'],
  template: '<input :value="value" :disabled="disabled" @input="$emit(\'update:value\', $event.target.value)">',
})
const select = defineComponent({
  props: ['value', 'disabled', 'options'],
  emits: ['update:value'],
  template: '<select :value="value" :disabled="disabled" @change="$emit(\'update:value\', $event.target.value)"><option v-for="option in options" :key="option.value" :value="option.value">{{ option.label }}</option></select>',
})

const wrappers: VueWrapper[] = []
let current: UiuxConfigData
let statusPhase = 'installed'

function modelStatus(directory = current.effective_model_dir) {
  return {
    phase: statusPhase,
    embedding_phase: 'unloaded',
    model_dir: directory,
    model_name: 'BGE',
    model_downloaded_bytes: 100,
    indexed_documents: 0,
    progress_percent: 100,
    runtime_ready: true,
    message: `目录状态 ${directory}`,
  }
}

function mountConfig(session?: UiuxEditSession) {
  const wrapper = mount(UiuxConfig, {
    props: { active: true, session },
    global: {
      stubs: {
        'n-scrollbar': slots,
        'n-spin': slots,
        'n-space': slots,
        'n-form-item': slots,
        'n-input-group': slots,
        'n-tooltip': slots,
        'n-alert': slots,
        'n-tag': slots,
        'n-button': button,
        'n-input': input,
        'n-select': select,
        'n-switch': true,
        'n-progress': true,
      },
    },
  })
  wrappers.push(wrapper)
  return wrapper
}

function action(wrapper: VueWrapper, label: string) {
  const found = wrapper.findAll('button').find(value => value.text() === label)
  if (!found)
    throw new Error(`缺少按钮：${label}`)
  return found
}

function calls(command: string) {
  return mocks.invoke.mock.calls.filter(([name]) => name === command)
}

function latestSession(wrapper: VueWrapper): UiuxEditSession {
  return wrapper.emitted('update:session')!.at(-1)![0] as UiuxEditSession
}

beforeEach(() => {
  vi.clearAllMocks()
  current = { knowledge_backend: 'auto', semantic_enabled: true, model_dir: 'C:/models/a', effective_model_dir: 'C:/models/a' }
  statusPhase = 'installed'
  mocks.invoke.mockImplementation(async (command, args) => {
    if (command === 'get_uiux_config')
      return { ...current }
    if (command === 'get_uiux_model_status')
      return modelStatus()
    if (command === 'set_uiux_config') {
      current = { ...args.config, effective_model_dir: args.config.model_dir || 'C:/models/default' }
      return { ...current }
    }
    if (command === 'start_uiux_model_download')
      return modelStatus(args.expectedModelDir)
    if (command === 'verify_uiux_model_integrity')
      return { valid: true, state: 'valid', model_dir: args.expectedModelDir, checked_at: '2026-09-21', message: '校验通过' }
    throw new Error(`意外命令：${command}`)
  })
})

afterEach(() => {
  for (const wrapper of wrappers.splice(0))
    wrapper.unmount()
  vi.useRealTimers()
})

describe('uiux 配置草稿与模型目标', () => {
  it('保存失败仍保留目录草稿和原始生效基线', async () => {
    const wrapper = mountConfig()
    await flushPromises()
    const defaultInvoke = mocks.invoke.getMockImplementation()!
    mocks.invoke.mockImplementation((command, args) => command === 'set_uiux_config'
      ? Promise.reject(new Error('磁盘写入失败'))
      : defaultInvoke(command, args))
    await wrapper.get('input').setValue('C:/models/b')
    await action(wrapper, '保存配置').trigger('click')
    await flushPromises()
    expect(latestSession(wrapper).draft.model_dir).toBe('C:/models/b')
    expect(latestSession(wrapper).saved.effective_model_dir).toBe('C:/models/a')
    expect(calls('set_uiux_config')[0][1].expectedConfig).toEqual(current)
    expect(mocks.error).toHaveBeenCalledWith(expect.stringContaining('磁盘写入失败'))
  })

  it('目录草稿阻止全部模型操作，策略草稿下载只使用已生效目录', async () => {
    statusPhase = 'missing'
    const wrapper = mountConfig()
    await flushPromises()
    await wrapper.get('input').setValue('C:/models/b')
    expect(action(wrapper, '下载模型').attributes('disabled')).toBeDefined()
    expect(action(wrapper, '重新校验').attributes('disabled')).toBeDefined()
    expect(wrapper.get('[aria-label="删除本地模型"]').attributes('disabled')).toBeDefined()
    await action(wrapper, '下载模型').trigger('click')
    expect(calls('start_uiux_model_download')).toHaveLength(0)
    await action(wrapper, '恢复已保存配置').trigger('click')
    await wrapper.get('select').setValue('local')
    await action(wrapper, '下载模型').trigger('click')
    await flushPromises()
    expect(calls('start_uiux_model_download')[0][1]).toEqual({ expectedModelDir: 'C:/models/a' })
    expect(calls('set_uiux_config')).toHaveLength(0)
    expect(latestSession(wrapper).draft.knowledge_backend).toBe('local')
    expect(latestSession(wrapper).saved.knowledge_backend).toBe('auto')
  })

  it('重新校验只调用校验命令，已安装状态不提示重新下载', async () => {
    const wrapper = mountConfig()
    await flushPromises()
    expect(wrapper.text()).toContain('已安装，按需加载')
    expect(wrapper.findAll('button').some(value => value.text() === '下载模型')).toBe(false)
    await action(wrapper, '重新校验').trigger('click')
    await flushPromises()
    expect(calls('verify_uiux_model_integrity')[0][1]).toEqual({ expectedModelDir: 'C:/models/a' })
    expect(calls('start_uiux_model_download')).toHaveLength(0)
    expect(calls('set_uiux_config')).toHaveLength(0)
    expect(wrapper.text()).toContain('校验通过')
  })

  it('共享推理已被 Sou 加载时，UIUX 已安装状态仍准确显示推理已加载', async () => {
    const defaultInvoke = mocks.invoke.getMockImplementation()!
    mocks.invoke.mockImplementation((command, args) => command === 'get_uiux_model_status'
      ? Promise.resolve({ ...modelStatus(), embedding_phase: 'ready' })
      : defaultInvoke(command, args))
    const wrapper = mountConfig()
    await flushPromises()
    expect(wrapper.text()).toContain('已安装，按需加载')
    expect(wrapper.text()).toContain('推理状态：已加载')
    expect(wrapper.text()).not.toContain('尚未加载或已闲置释放')
  })

  it('校验失败后提供固定目录修复下载，不要求先删除', async () => {
    const defaultInvoke = mocks.invoke.getMockImplementation()!
    mocks.invoke.mockImplementation((command, args) => command === 'verify_uiux_model_integrity'
      ? Promise.resolve({ valid: false, state: 'invalid', model_dir: args.expectedModelDir, checked_at: '2026-09-21', message: '模型校验不一致' })
      : defaultInvoke(command, args))
    const wrapper = mountConfig()
    await flushPromises()
    await action(wrapper, '重新校验').trigger('click')
    await flushPromises()
    await action(wrapper, '修复下载').trigger('click')
    await flushPromises()
    expect(calls('start_uiux_model_download')[0][1]).toEqual({ expectedModelDir: 'C:/models/a' })
    expect(calls('remove_uiux_model')).toHaveLength(0)
    expect(calls('set_uiux_config')).toHaveLength(0)
  })

  it('删除确认展示固定目录及共享影响，确认后保持同一目标', async () => {
    const wrapper = mountConfig()
    await flushPromises()
    await wrapper.get('[aria-label="删除本地模型"]').trigger('click')
    const confirmation = mocks.warning.mock.calls[0][0]
    expect(confirmation.content).toContain('C:/models/a')
    expect(confirmation.content).toContain('UIUX 与 Sou 共享')
    const defaultInvoke = mocks.invoke.getMockImplementation()!
    mocks.invoke.mockImplementation((command, args) => command === 'remove_uiux_model'
      ? Promise.resolve(modelStatus(args.expectedModelDir))
      : defaultInvoke(command, args))
    await confirmation.onPositiveClick()
    await flushPromises()
    expect(calls('remove_uiux_model')[0][1]).toEqual({ expectedModelDir: 'C:/models/a' })
  })

  it('离开页面后旧删除确认失效，不再触发模型操作', async () => {
    const wrapper = mountConfig()
    await flushPromises()
    await wrapper.get('[aria-label="删除本地模型"]').trigger('click')
    const confirmation = mocks.warning.mock.calls[0][0]
    wrapper.unmount()
    await confirmation.onPositiveClick()
    expect(calls('remove_uiux_model')).toHaveLength(0)
  })

  it('重新挂载保留草稿，外部变更显示冲突直到显式加载', async () => {
    const original = mountConfig()
    await flushPromises()
    await original.get('input').setValue('C:/models/b')
    const session = latestSession(original)
    original.unmount()
    current = { ...current, knowledge_backend: 'local' }
    const reopened = mountConfig(session)
    await flushPromises()
    expect((reopened.get('input').element as HTMLInputElement).value).toBe('C:/models/b')
    expect(reopened.text()).toContain('配置已在其他入口更新')
    expect(action(reopened, '保存配置').attributes('disabled')).toBeDefined()
    await action(reopened, '加载当前配置').trigger('click')
    await flushPromises()
    expect((reopened.get('input').element as HTMLInputElement).value).toBe('C:/models/a')
    expect(latestSession(reopened).saved.knowledge_backend).toBe('local')
    expect(calls('set_uiux_config')).toHaveLength(0)
  })

  it('保存成功后状态读取失败单独显示，不回滚已保存结果', async () => {
    const wrapper = mountConfig()
    await flushPromises()
    const defaultInvoke = mocks.invoke.getMockImplementation()!
    mocks.invoke.mockImplementation((command, args) => command === 'get_uiux_model_status'
      ? Promise.reject(new Error('状态通道暂不可用'))
      : defaultInvoke(command, args))
    await wrapper.get('input').setValue('C:/models/b')
    await action(wrapper, '保存配置').trigger('click')
    await flushPromises()
    expect(latestSession(wrapper).saved.effective_model_dir).toBe('C:/models/b')
    expect(mocks.success).toHaveBeenCalledWith('UIUX 配置已保存')
    expect(mocks.error).not.toHaveBeenCalled()
    expect(wrapper.text()).toContain('状态更新失败')
  })
})

describe('uiux 状态请求时序', () => {
  it('状态请求单次在途，旧目录响应退出后再查询新目录', async () => {
    const wrapper = mountConfig()
    await flushPromises()
    const pending = deferred<ReturnType<typeof modelStatus>>()
    const defaultInvoke = mocks.invoke.getMockImplementation()!
    let delayNextStatus = true
    mocks.invoke.mockImplementation((command, args) => {
      if (command === 'get_uiux_model_status' && delayNextStatus) {
        delayNextStatus = false
        return pending.promise
      }
      return defaultInvoke(command, args)
    })
    await wrapper.get('[aria-label="刷新模型状态"]').trigger('click')
    await wrapper.get('[aria-label="刷新模型状态"]').trigger('click')
    await wrapper.get('input').setValue('C:/models/b')
    await action(wrapper, '保存配置').trigger('click')
    await flushPromises()
    expect(calls('get_uiux_model_status')).toHaveLength(2)
    expect(wrapper.text()).not.toContain('目录状态 C:/models/a')
    pending.resolve(modelStatus('C:/models/a'))
    await flushPromises()
    expect(calls('get_uiux_model_status')).toHaveLength(3)
    expect(wrapper.text()).toContain('目录状态 C:/models/b')
    expect(wrapper.text()).not.toContain('目录状态 C:/models/a')
  })

  it('隐藏时忽略迟到响应，重新激活恢复刷新且不并行请求', async () => {
    const wrapper = mountConfig()
    await flushPromises()
    const pending = deferred<ReturnType<typeof modelStatus>>()
    mocks.invoke.mockImplementationOnce(() => pending.promise)
    await wrapper.get('[aria-label="刷新模型状态"]').trigger('click')
    await wrapper.setProps({ active: false })
    pending.resolve({ ...modelStatus(), message: '迟到状态' })
    await flushPromises()
    expect(wrapper.text()).not.toContain('迟到状态')
    await wrapper.setProps({ active: true })
    await flushPromises()
    expect(calls('get_uiux_model_status')).toHaveLength(3)
  })
})
