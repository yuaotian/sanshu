import type { PlanSnapshot } from '../../../types/plan'
import { flushPromises, shallowMount } from '@vue/test-utils'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import PlanPanel from '../PlanPanel.vue'

const tauri = vi.hoisted(() => ({
  invoke: vi.fn(),
  listen: vi.fn(),
}))

vi.mock('@tauri-apps/api/core', () => ({
  invoke: tauri.invoke,
}))

vi.mock('@tauri-apps/api/event', () => ({
  listen: tauri.listen,
}))

interface Deferred<T> {
  promise: Promise<T>
  resolve: (value: T | PromiseLike<T>) => void
  reject: (reason?: unknown) => void
}

function deferred<T>(): Deferred<T> {
  let resolve!: Deferred<T>['resolve']
  let reject!: Deferred<T>['reject']
  const promise = new Promise<T>((resolvePromise, rejectPromise) => {
    resolve = resolvePromise
    reject = rejectPromise
  })
  return { promise, resolve, reject }
}

function emptySnapshot(workspace: string): PlanSnapshot {
  return {
    action: 'get',
    workspace,
    changed: false,
    items: [],
    summary: {
      completed: 0,
      total: 0,
      all_completed: false,
    },
  }
}

function completedSnapshot(workspace: string): PlanSnapshot {
  return {
    action: 'get',
    workspace,
    changed: false,
    items: [{ id: 'step-1', text: '完成修复', status: 'completed' }],
    summary: {
      completed: 1,
      total: 1,
      all_completed: true,
    },
  }
}

function pendingSnapshot(workspace: string): PlanSnapshot {
  return {
    action: 'get',
    workspace,
    changed: false,
    items: [
      { id: 'step-1', text: '旧步骤一', status: 'pending' },
      { id: 'step-2', text: '旧步骤二', status: 'pending' },
    ],
    summary: {
      completed: 0,
      total: 2,
      all_completed: false,
    },
  }
}

function mountPanel(workspace = 'C:/workspace-a') {
  return shallowMount(PlanPanel, {
    props: { workspace },
    global: {
      stubs: {
        'n-button': true,
        'n-switch': true,
        'n-tooltip': true,
      },
    },
  })
}

function commandCalls(command: string) {
  return tauri.invoke.mock.calls.filter(([name]) => name === command)
}

describe('plan panel watcher 生命周期', () => {
  beforeEach(() => {
    localStorage.clear()
    tauri.invoke.mockReset()
    tauri.listen.mockReset()
    tauri.listen.mockResolvedValue(vi.fn())
    tauri.invoke.mockImplementation((command: string, args?: { workspace?: string }) => {
      if (command === 'get_plan_snapshot')
        return Promise.resolve(emptySnapshot(args?.workspace ?? ''))
      return Promise.resolve()
    })
  })

  it('卸载后才完成监听注册时立即释放监听器', async () => {
    const listener = deferred<() => void>()
    const unlisten = vi.fn()
    tauri.listen.mockReturnValueOnce(listener.promise)

    const wrapper = mountPanel()
    await flushPromises()
    wrapper.unmount()

    listener.resolve(unlisten)
    await flushPromises()

    expect(unlisten).toHaveBeenCalledOnce()
    expect(commandCalls('start_plan_watch')).toHaveLength(0)
    expect(commandCalls('get_plan_snapshot')).toHaveLength(0)
  })

  it('卸载后才完成 watcher 启动时补执行停止', async () => {
    const start = deferred<void>()
    tauri.invoke.mockImplementation((command: string, args?: { workspace?: string }) => {
      if (command === 'start_plan_watch')
        return start.promise
      if (command === 'get_plan_snapshot')
        return Promise.resolve(emptySnapshot(args?.workspace ?? ''))
      return Promise.resolve()
    })

    const wrapper = mountPanel()
    await flushPromises()
    expect(commandCalls('start_plan_watch')).toHaveLength(1)

    wrapper.unmount()
    await flushPromises()
    expect(commandCalls('stop_plan_watch')).toHaveLength(1)

    start.resolve(undefined)
    await flushPromises()

    expect(commandCalls('stop_plan_watch')).toHaveLength(2)
    expect(commandCalls('get_plan_snapshot')).toHaveLength(0)
  })

  it('快速切换工作区时只启动最后一个待处理工作区', async () => {
    const startA = deferred<void>()
    tauri.invoke.mockImplementation((command: string, args?: { workspace?: string }) => {
      if (command === 'start_plan_watch' && args?.workspace === 'C:/workspace-a')
        return startA.promise
      if (command === 'get_plan_snapshot')
        return Promise.resolve(emptySnapshot(args?.workspace ?? ''))
      return Promise.resolve()
    })

    const wrapper = mountPanel()
    await flushPromises()
    await wrapper.setProps({ workspace: 'C:/workspace-b' })
    await wrapper.setProps({ workspace: 'C:/workspace-c' })
    await flushPromises()

    startA.resolve(undefined)
    await flushPromises()

    const startedWorkspaces = commandCalls('start_plan_watch').map(([, args]) => args?.workspace)
    expect(startedWorkspaces).toEqual(['C:/workspace-a', 'C:/workspace-c'])
    expect(commandCalls('get_plan_snapshot').map(([, args]) => args?.workspace)).toEqual(['C:/workspace-c'])

    wrapper.unmount()
    await flushPromises()
  })

  it('正常路径按监听、启动、读取顺序执行并在卸载时清理', async () => {
    const order: string[] = []
    const unlisten = vi.fn(() => order.push('unlisten'))
    tauri.listen.mockImplementation(() => {
      order.push('listen')
      return Promise.resolve(unlisten)
    })
    tauri.invoke.mockImplementation((command: string, args?: { workspace?: string }) => {
      order.push(command)
      if (command === 'get_plan_snapshot')
        return Promise.resolve(emptySnapshot(args?.workspace ?? ''))
      return Promise.resolve()
    })

    const wrapper = mountPanel()
    await flushPromises()

    expect(order.slice(0, 3)).toEqual(['listen', 'start_plan_watch', 'get_plan_snapshot'])

    wrapper.unmount()
    await flushPromises()
    expect(unlisten).toHaveBeenCalledOnce()
    expect(commandCalls('stop_plan_watch')).toHaveLength(1)
  })

  it('迟到快照不会覆盖最新工作区状态', async () => {
    const snapshotA = deferred<PlanSnapshot>()
    tauri.invoke.mockImplementation((command: string, args?: { workspace?: string }) => {
      if (command === 'get_plan_snapshot' && args?.workspace === 'C:/workspace-a')
        return snapshotA.promise
      if (command === 'get_plan_snapshot')
        return Promise.resolve(completedSnapshot(args?.workspace ?? ''))
      return Promise.resolve()
    })

    const wrapper = mountPanel()
    await flushPromises()
    await wrapper.setProps({ workspace: 'C:/workspace-b' })
    await flushPromises()
    expect(wrapper.text()).toContain('1/1')

    snapshotA.resolve(pendingSnapshot('C:/workspace-a'))
    await flushPromises()
    expect(wrapper.text()).toContain('1/1')
    expect(wrapper.text()).not.toContain('(0/2)')

    wrapper.unmount()
    await flushPromises()
  })

  it('折叠态保留可理解的状态文本并归零原生按钮外观', async () => {
    tauri.invoke.mockImplementation((command: string, args?: { workspace?: string }) => {
      if (command === 'get_plan_snapshot')
        return Promise.resolve(completedSnapshot(args?.workspace ?? ''))
      return Promise.resolve()
    })

    const wrapper = mountPanel()
    await flushPromises()

    expect(wrapper.text()).toContain('已完成')
    expect(wrapper.text()).toContain('1/1')
    expect(wrapper.find('button[aria-expanded]').classes()).toContain('plan-reset-button')
    expect(wrapper.find('button[aria-label="执行计划设置"]').classes()).toContain('plan-reset-button')

    wrapper.unmount()
    await flushPromises()
  })

  it('设置弹层关联触发器并在点击正文后关闭', async () => {
    tauri.invoke.mockImplementation((command: string, args?: { workspace?: string }) => {
      if (command === 'get_plan_snapshot')
        return Promise.resolve(pendingSnapshot(args?.workspace ?? ''))
      return Promise.resolve()
    })

    const wrapper = mountPanel()
    await flushPromises()
    await wrapper.find('button[aria-expanded]').trigger('click')
    const settingsTrigger = wrapper.find('button[aria-label="执行计划设置"]')
    await settingsTrigger.trigger('click')

    expect(settingsTrigger.attributes('aria-controls')).toBe('plan-panel-settings')
    expect(wrapper.find('#plan-panel-settings').attributes('aria-labelledby')).toBe('plan-panel-settings-label')

    await wrapper.find('.plan-item').trigger('pointerdown')
    await wrapper.find('.plan-item').trigger('click')
    expect(wrapper.find('#plan-panel-settings').exists()).toBe(false)

    wrapper.unmount()
    await flushPromises()
  })

  it('覆盖空计划、读取失败和实时中断状态', async () => {
    const wrapper = mountPanel()
    await flushPromises()
    await wrapper.find('button[aria-expanded]').trigger('click')
    expect(wrapper.text()).toContain('当前工作区暂无执行计划')
    wrapper.unmount()
    await flushPromises()

    tauri.invoke.mockImplementation((command: string) => {
      if (command === 'get_plan_snapshot')
        return Promise.reject(new Error('读取失败'))
      return Promise.resolve()
    })
    const readErrorWrapper = mountPanel()
    await flushPromises()
    await readErrorWrapper.find('button[aria-expanded]').trigger('click')
    expect(readErrorWrapper.text()).toContain('计划读取失败')
    readErrorWrapper.unmount()
    await flushPromises()

    tauri.invoke.mockImplementation((command: string, args?: { workspace?: string }) => {
      if (command === 'get_plan_snapshot')
        return Promise.resolve(pendingSnapshot(args?.workspace ?? ''))
      return Promise.resolve()
    })
    tauri.listen.mockRejectedValueOnce(new Error('监听失败'))
    const realtimeErrorWrapper = mountPanel()
    await flushPromises()
    await realtimeErrorWrapper.find('button[aria-expanded]').trigger('click')
    expect(realtimeErrorWrapper.text()).toContain('实时刷新暂不可用')
    realtimeErrorWrapper.unmount()
    await flushPromises()
  })

  it('展开后可通过回车保存工作区本地备注', async () => {
    tauri.invoke.mockImplementation((command: string, args?: { workspace?: string }) => {
      if (command === 'get_plan_snapshot')
        return Promise.resolve(completedSnapshot(args?.workspace ?? ''))
      return Promise.resolve()
    })

    const wrapper = mountPanel()
    await flushPromises()
    await wrapper.find('button[aria-expanded]').trigger('click')

    const input = wrapper.find('input[aria-label="添加执行计划本地备注"]')
    await input.setValue('保留当前验收结论')
    await input.trigger('keydown.enter')

    expect(wrapper.text()).toContain('保留当前验收结论')
    expect(localStorage.getItem('popup-plan-panel-note:C%3A%2Fworkspace-a')).toBe('保留当前验收结论')

    wrapper.unmount()
    await flushPromises()
  })

  it('已有备注可一键载入、替换或取消编辑', async () => {
    localStorage.setItem('popup-plan-panel-note:C%3A%2Fworkspace-a', '原始备注')
    tauri.invoke.mockImplementation((command: string, args?: { workspace?: string }) => {
      if (command === 'get_plan_snapshot')
        return Promise.resolve(completedSnapshot(args?.workspace ?? ''))
      return Promise.resolve()
    })

    const wrapper = mountPanel()
    await flushPromises()
    await wrapper.find('button[aria-expanded]').trigger('click')

    const input = wrapper.find('input[aria-label="添加执行计划本地备注"]')
    await wrapper.find('button[aria-label="编辑本地备注"]').trigger('click')
    expect((input.element as HTMLInputElement).value).toBe('原始备注')

    await input.setValue('替换后的备注')
    await input.trigger('keydown.esc')
    expect((input.element as HTMLInputElement).value).toBe('')
    expect(localStorage.getItem('popup-plan-panel-note:C%3A%2Fworkspace-a')).toBe('原始备注')

    await wrapper.find('button[aria-label="编辑本地备注"]').trigger('click')
    await input.setValue('替换后的备注')
    await input.trigger('keydown.enter')
    expect(localStorage.getItem('popup-plan-panel-note:C%3A%2Fworkspace-a')).toBe('替换后的备注')
    expect(wrapper.text()).toContain('替换后的备注')

    await wrapper.find('button[aria-label="清除本地备注"]').trigger('click')
    expect(localStorage.getItem('popup-plan-panel-note:C%3A%2Fworkspace-a')).toBeNull()

    wrapper.unmount()
    await flushPromises()
  })
})
