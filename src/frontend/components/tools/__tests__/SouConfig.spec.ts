import type { VueWrapper } from '@vue/test-utils'
import { flushPromises, shallowMount } from '@vue/test-utils'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import SouConfig from '../SouConfig.vue'

const tauri = vi.hoisted(() => ({ invoke: vi.fn(), listen: vi.fn() }))
const feedback = vi.hoisted(() => ({
  info: vi.fn(),
  warning: vi.fn(),
  error: vi.fn(),
  success: vi.fn(),
  loading: vi.fn(),
}))
vi.mock('@tauri-apps/api/core', () => ({ invoke: tauri.invoke }))
vi.mock('@tauri-apps/api/event', () => ({ listen: tauri.listen }))
vi.mock('naive-ui', () => ({ useMessage: () => feedback, useDialog: () => ({ warning: vi.fn() }) }))
vi.mock('../../../composables/useAcemcpSync', () => ({
  useAcemcpSync: () => ({
    autoIndexEnabled: false,
    fetchAutoIndexEnabled: vi.fn(),
    setAutoIndexEnabled: vi.fn(),
    fetchWatchingProjects: vi.fn(),
  }),
}))

function deferred<T>() {
  let resolve!: (value: T) => void
  let reject!: (reason: Error) => void
  const promise = new Promise<T>((res, rej) => {
    resolve = res
    reject = rej
  })
  return { promise, resolve, reject }
}

function modelStatus(modelDir = 'C:/saved/model', phase = 'installed') {
  return {
    model_dir: modelDir,
    phase,
    embedding_phase: phase === 'ready' ? 'ready' : 'unloaded',
    execution_provider: 'cpu',
    downloaded_bytes: 100,
    total_bytes: 100,
    progress_percent: 100,
    message: '模型资产齐备，查询时按需加载',
  }
}

interface ModelHarness {
  config: { local_embedding_model_dir: string, sou_default_backend: string, sou_local_semantic_mode: string }
  embeddingModelStatus: ReturnType<typeof modelStatus> | null
  embeddingStatusReadError: string
  embeddingPhaseLabel: string
  embeddingAssetsInstalled: boolean
  embeddingNeedsRepair: boolean
  embeddingTaskActive: boolean
  embeddingExecutionProviderLabel: string
  effectiveEmbeddingModelDir: string
  embeddingIntegrity: { model_dir: string, valid: boolean } | null
  refreshEmbeddingModelStatus: (showFeedback?: boolean) => Promise<void>
  installEmbeddingModel: () => Promise<void>
  verifyEmbeddingIntegrity: () => Promise<void>
  rememberStorageConfig: () => void
  saveConfig: (showFeedback?: boolean) => Promise<boolean>
}

const wrappers: VueWrapper[] = []
let persistedDirectory: string
let statusResponse: () => Promise<ReturnType<typeof modelStatus>>

async function mountConfig() {
  const wrapper = shallowMount(SouConfig, {
    props: { active: true },
    global: {
      stubs: { 'n-tabs': true },
      config: { warnHandler: () => {} },
    },
  })
  wrappers.push(wrapper)
  await flushPromises()
  return { wrapper, vm: wrapper.vm as unknown as ModelHarness }
}

function calls(command: string) {
  return tauri.invoke.mock.calls.filter(([name]) => name === command)
}

describe('sou 共享模型操作与状态生命周期', () => {
  beforeEach(() => {
    vi.useFakeTimers()
    vi.clearAllMocks()
    persistedDirectory = 'C:/saved/model'
    statusResponse = async () => modelStatus(persistedDirectory)
    tauri.listen.mockResolvedValue(vi.fn())
    tauri.invoke.mockImplementation(async (command: string, args?: { expectedModelDir?: string }) => {
      if (command === 'get_acemcp_config') {
        return {
          batch_size: 10,
          max_lines_per_blob: 800,
          text_extensions: [],
          exclude_patterns: [],
          local_embedding_model_dir: persistedDirectory,
          effective_local_embedding_model_dir: persistedDirectory,
          sou_local_semantic_mode: 'balanced',
          fast_context_api_key: 'fixture-key',
        }
      }
      if (command === 'get_uiux_model_status')
        return statusResponse()
      if (command === 'get_all_acemcp_index_status')
        return { projects: {} }
      if (command === 'get_sou_reranker_model_status')
        return { phase: 'missing' }
      if (command === 'get_sou_resource_usage')
        return { sampled_at: '', message: '' }
      if (command === 'start_uiux_model_download')
        return modelStatus(args?.expectedModelDir, 'downloading')
      if (command === 'verify_uiux_model_integrity')
        return { model_dir: args?.expectedModelDir, valid: true, message: '完整' }
      return null
    })
  })

  afterEach(() => {
    for (const wrapper of wrappers.splice(0))
      wrapper.unmount()
    vi.useRealTimers()
  })

  it('已安装显示按需加载并停止快速轮询，不显示默认 CPU 为实际推理设备', async () => {
    const { vm } = await mountConfig()
    expect(vm.embeddingPhaseLabel).toBe('已安装，按需加载')
    expect(vm.embeddingAssetsInstalled).toBe(true)
    expect(vm.embeddingTaskActive).toBe(false)
    expect(vm.embeddingExecutionProviderLabel).toBe('按需加载')
    const count = calls('get_uiux_model_status').length
    await vi.advanceTimersByTimeAsync(3500)
    expect(calls('get_uiux_model_status')).toHaveLength(count)
  })

  it('下载和校验只使用后端确认目录，不隐式保存检索策略草稿', async () => {
    const { vm } = await mountConfig()
    vm.config.sou_default_backend = 'fast_context'
    // 中文说明：模拟选择器展示路径变化，操作目标仍由后端状态确认。
    vm.effectiveEmbeddingModelDir = 'C:/display/only'
    await vm.verifyEmbeddingIntegrity()
    await vm.installEmbeddingModel()
    expect(calls('verify_uiux_model_integrity')[0][1]).toEqual({ expectedModelDir: 'C:/saved/model' })
    expect(calls('start_uiux_model_download')[0][1]).toEqual({ expectedModelDir: 'C:/saved/model' })
    expect(calls('save_acemcp_config')).toHaveLength(0)
    expect(vm.config.sou_default_backend).toBe('fast_context')
  })

  it('目录草稿未保存时函数入口也禁止下载和校验', async () => {
    const { vm } = await mountConfig()
    vm.config.local_embedding_model_dir = 'C:/unsaved/model'
    await vm.installEmbeddingModel()
    await vm.verifyEmbeddingIntegrity()
    expect(calls('start_uiux_model_download')).toHaveLength(0)
    expect(calls('verify_uiux_model_integrity')).toHaveLength(0)
    expect(calls('save_acemcp_config')).toHaveLength(0)
    expect(feedback.warning).toHaveBeenCalledWith('共享模型目录尚未保存，请先显式保存配置后再操作')
  })

  it('已安装资产校验损坏时保留当前目录的修复下载入口', async () => {
    const { vm } = await mountConfig()
    vm.embeddingIntegrity = { model_dir: 'C:/other/model', valid: false }
    expect(vm.embeddingNeedsRepair).toBe(false)
    vm.embeddingIntegrity = { model_dir: 'C:/saved/model', valid: false }
    expect(vm.embeddingAssetsInstalled).toBe(true)
    expect(vm.embeddingNeedsRepair).toBe(true)
    await vm.installEmbeddingModel()
    expect(calls('start_uiux_model_download')[0][1]).toEqual({ expectedModelDir: 'C:/saved/model' })
    expect(calls('save_acemcp_config')).toHaveLength(0)
  })

  it('目录变化使旧响应失效，补刷新始终最多一个状态请求在途', async () => {
    const { vm } = await mountConfig()
    const old = deferred<ReturnType<typeof modelStatus>>()
    const fresh = deferred<ReturnType<typeof modelStatus>>()
    let active = 0
    let maxActive = 0
    let requests = 0
    statusResponse = async () => {
      active += 1
      maxActive = Math.max(maxActive, active)
      try {
        return await (++requests === 1 ? old.promise : fresh.promise)
      }
      finally {
        active -= 1
      }
    }
    const pending = vm.refreshEmbeddingModelStatus()
    vm.config.local_embedding_model_dir = 'C:/new/model'
    persistedDirectory = 'C:/new/model'
    await vm.saveConfig(false)
    await vm.refreshEmbeddingModelStatus()
    expect(requests).toBe(1)
    old.resolve(modelStatus('C:/saved/model', 'loading'))
    await pending
    await flushPromises()
    expect(vm.embeddingModelStatus).toBeNull()
    expect(requests).toBe(2)
    fresh.resolve(modelStatus('C:/new/model'))
    await flushPromises()
    expect(vm.embeddingModelStatus?.model_dir).toBe('C:/new/model')
    expect(maxActive).toBe(1)
  })

  it('切走后忽略旧响应，重新激活再读取有效状态', async () => {
    const { vm, wrapper } = await mountConfig()
    const old = deferred<ReturnType<typeof modelStatus>>()
    statusResponse = () => old.promise
    const pending = vm.refreshEmbeddingModelStatus()
    await wrapper.setProps({ active: false })
    old.resolve(modelStatus('C:/stale/model', 'loading'))
    await pending
    expect(vm.embeddingModelStatus?.model_dir).toBe('C:/saved/model')
    statusResponse = async () => modelStatus(persistedDirectory)
    await wrapper.setProps({ active: true })
    await flushPromises()
    expect(vm.embeddingModelStatus?.phase).toBe('installed')
  })

  it('状态失败保留最近结果和可见错误，重试成功清除错误', async () => {
    const { vm } = await mountConfig()
    statusResponse = async () => {
      throw new Error('fixture status failure')
    }
    await vm.refreshEmbeddingModelStatus(true)
    expect(vm.embeddingModelStatus?.model_dir).toBe('C:/saved/model')
    expect(vm.embeddingStatusReadError).toContain('fixture status failure')
    await vm.installEmbeddingModel()
    expect(calls('start_uiux_model_download')).toHaveLength(0)
    statusResponse = async () => modelStatus(persistedDirectory)
    await vm.refreshEmbeddingModelStatus(true)
    expect(vm.embeddingStatusReadError).toBe('')
  })

  it('卸载后到达的请求不写回状态或重新启动轮询', async () => {
    const { vm, wrapper } = await mountConfig()
    const old = deferred<ReturnType<typeof modelStatus>>()
    statusResponse = () => old.promise
    const pending = vm.refreshEmbeddingModelStatus()
    wrapper.unmount()
    old.resolve(modelStatus('C:/late/model', 'loading'))
    await pending
    const count = calls('get_uiux_model_status').length
    await vi.advanceTimersByTimeAsync(2500)
    expect(calls('get_uiux_model_status')).toHaveLength(count)
    expect(vm.embeddingModelStatus?.model_dir).toBe('C:/saved/model')
  })

  it('外部更改目录后状态响应不静默改写操作目标或本页草稿', async () => {
    const { vm } = await mountConfig()
    vm.config.sou_default_backend = 'fast_context'
    statusResponse = async () => modelStatus('C:/external/model')
    await vm.refreshEmbeddingModelStatus(true)
    expect(vm.embeddingModelStatus?.model_dir).toBe('C:/saved/model')
    expect(vm.embeddingStatusReadError).toContain('重新加载已保存配置')
    await vm.installEmbeddingModel()
    await vm.verifyEmbeddingIntegrity()
    expect(calls('start_uiux_model_download')).toHaveLength(0)
    expect(calls('verify_uiux_model_integrity')).toHaveLength(0)
    expect(calls('save_acemcp_config')).toHaveLength(0)
    expect(vm.config.local_embedding_model_dir).toBe('C:/saved/model')
    expect(vm.config.sou_default_backend).toBe('fast_context')
  })
})
