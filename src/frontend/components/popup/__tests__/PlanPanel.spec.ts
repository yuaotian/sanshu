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
    expect(wrapper.text()).toContain('(1/1)')

    snapshotA.resolve(pendingSnapshot('C:/workspace-a'))
    await flushPromises()
    expect(wrapper.text()).toContain('(1/1)')
    expect(wrapper.text()).not.toContain('(0/2)')

    wrapper.unmount()
    await flushPromises()
  })
})
