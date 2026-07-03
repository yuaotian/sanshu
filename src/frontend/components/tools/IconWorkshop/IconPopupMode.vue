<script setup lang="ts">
/**
 * 图标工坊 - 弹窗模式（MCP tu 工具入口）
 * 纯编排层：左侧搜索结果网格 + 右侧固定编辑抽屉 + 保存进度覆盖层 + 右键菜单
 * 弹窗生命周期（响应构建、退出）唯一收敛于此组件；
 * 响应采用结构化格式（status: saved/cancelled），与 MCP 侧 IconPopupResponse 对齐
 */
import { invoke } from '@tauri-apps/api/core'
import { useMessage } from 'naive-ui'
import { computed, ref } from 'vue'
import { useIconEditor } from '../../../composables/useIconEditor'
import { useIconSearch } from '../../../composables/useIconSearch'
import type { IconFormat, IconItem, IconSaveItem, IconSaveRequest, IconSaveResult } from '../../../types/icon'
import IconContextMenu from './IconContextMenu.vue'
import IconEditorDrawer from './IconEditorDrawer.vue'
import IconWorkshop from './IconWorkshop.vue'
import SaveProgressOverlay from './SaveProgressOverlay.vue'

interface Props {
  initialQuery?: string
  initialStyle?: string
  initialSavePath?: string
  projectRoot?: string
}

const props = defineProps<Props>()

const message = useMessage()
const { saveIcons } = useIconSearch()

// ============ 保存进度状态 ============
const isSaving = ref(false)
const saveProgress = ref(0)
const savingIconName = ref('')
const saveSummary = ref<IconSaveResult | null>(null)
const saveError = ref<string | null>(null)
const needsConfirm = ref(false)
const pendingResponse = ref<any>(null)

// ============ 选择与编辑状态 ============
const selectedIcons = ref<IconItem[]>([])
const editorOpen = ref(false)

// SVG 编辑器实例（状态由 useIconEditor 统一管理，抽屉与右键菜单共享）
const editor = useIconEditor(selectedIcons)

// ============ 右键菜单状态 ============
const contextMenuVisible = ref(false)
const contextMenuPosition = ref({ x: 0, y: 0 })
const contextMenuIcon = ref<IconItem | null>(null)

const showProgressOverlay = computed(() => isSaving.value || needsConfirm.value)

// ============ 选择与编辑器交互 ============

function handleSelectionChange(icons: IconItem[]) {
  selectedIcons.value = icons
}

// 双击图标：选中并展开编辑抽屉
function handleIconDblClick(icon: IconItem) {
  if (!selectedIcons.value.some(i => i.id === icon.id)) {
    selectedIcons.value = [...selectedIcons.value, icon]
  }
  editor.activeIconId.value = icon.id
  editorOpen.value = true
}

// 右键图标：显示上下文菜单
function handleIconContextMenu(icon: IconItem, event: MouseEvent) {
  contextMenuIcon.value = icon
  contextMenuPosition.value = { x: event.clientX, y: event.clientY }
  contextMenuVisible.value = true
}

function closeContextMenu() {
  contextMenuVisible.value = false
  contextMenuIcon.value = null
}

// 右键菜单：打开编辑抽屉
function contextMenuOpenEditor() {
  if (!contextMenuIcon.value)
    return
  handleIconDblClick(contextMenuIcon.value)
  closeContextMenu()
}

// 右键菜单：复制 SVG（优先编辑后的内容）
async function contextMenuCopySvg() {
  if (!contextMenuIcon.value)
    return
  const icon = contextMenuIcon.value
  try {
    const svgContent = editor.getEditedSvg(icon) || icon.svgContent
    if (svgContent) {
      await navigator.clipboard.writeText(svgContent)
      message.success(`已复制 ${icon.name} 的 SVG`)
    }
    else {
      message.warning('暂无可复制的 SVG 内容')
    }
  }
  catch (error) {
    console.error('复制失败:', error)
    message.error('复制失败')
  }
  closeContextMenu()
}

// ============ 保存流程 ============

/**
 * 逐图标批量保存（用于进度反馈与当前图标提示），
 * 完成后构建结构化响应等待用户确认关闭
 */
async function startSave(request: IconSaveRequest, isEditorSave = false) {
  if (isSaving.value)
    return

  isSaving.value = true
  saveProgress.value = 0
  savingIconName.value = ''
  saveError.value = null
  needsConfirm.value = false
  pendingResponse.value = null
  saveSummary.value = null

  const items: IconSaveItem[] = []
  const total = request.icons.length

  try {
    for (let index = 0; index < request.icons.length; index++) {
      const icon = request.icons[index]
      const iconForSave = editor.buildIconForSave(icon, isEditorSave)
      savingIconName.value = iconForSave.name

      const singleRequest: IconSaveRequest = {
        ...request,
        icons: [iconForSave],
      }

      const result = await saveIcons(singleRequest)
      if (result?.items?.length) {
        items.push(result.items[0])
      }
      else {
        const failureMessage = `${iconForSave.name} 保存失败`
        items.push({
          id: iconForSave.id,
          name: iconForSave.name,
          success: false,
          savedPaths: [],
          error: failureMessage,
        })
        if (!saveError.value)
          saveError.value = failureMessage
      }

      saveProgress.value = Math.round(((index + 1) / total) * 100)
    }
  }
  catch (error) {
    console.error('保存图标失败:', error)
    saveError.value = String(error)
  }
  finally {
    isSaving.value = false
  }

  const successCount = items.filter(item => item.success).length
  const failedCount = items.length - successCount

  saveSummary.value = {
    items,
    successCount,
    failedCount,
    savePath: request.savePath,
  }

  const savedNames = items.filter(item => item.success).map(item => item.name)
  const failedItems = items.filter(item => !item.success)
  const failureReason = saveError.value
    || failedItems.map(item => `${item.name}: ${item.error || '保存失败'}`).join('; ')

  // 结构化响应：保存失败必须返回 error，避免 MCP 误判为“未选择图标”
  pendingResponse.value = failureReason
    ? {
        status: 'error',
        saved_count: successCount,
        save_path: request.savePath,
        saved_names: savedNames,
        error: failureReason,
      }
    : {
        status: 'saved',
        saved_count: successCount,
        save_path: request.savePath,
        saved_names: savedNames,
        error: null,
      }

  needsConfirm.value = true
}

// 结果网格"保存选中"入口
async function handlePopupSave(request: IconSaveRequest) {
  if (!request.icons.length) {
    message.warning('没有可保存的图标')
    return
  }
  await startSave(request, false)
}

// 编辑抽屉"保存"入口（附时间戳文件名，避免覆盖原图标）
async function handleEditorSave(payload: { savePath: string, format: IconFormat, pngSize?: number }) {
  const icon = editor.activeIcon.value
  if (!icon) {
    message.warning('请先选择要编辑的图标')
    return
  }
  await startSave({
    icons: [icon],
    savePath: payload.savePath,
    format: payload.format,
    pngSize: payload.pngSize,
  }, true)
}

// 用户确认保存结果后发送响应并退出
async function handleConfirmClose() {
  if (!pendingResponse.value)
    return
  try {
    await invoke('send_mcp_response', { response: pendingResponse.value })
    await invoke('exit_app')
  }
  catch (error) {
    console.error('完成确认失败:', error)
    message.error('关闭失败，请重试')
  }
}

// 用户取消：发送 cancelled 状态并退出
async function handleCancel() {
  try {
    await invoke('send_mcp_response', { response: { status: 'cancelled' } })
    await invoke('exit_app')
  }
  catch (error) {
    console.error('取消图标弹窗失败:', error)
    await invoke('exit_app')
  }
}
</script>

<template>
  <div class="h-screen flex flex-col bg-surface text-on-surface">
    <!-- 顶部导航栏 -->
    <div class="flex-shrink-0 h-14 border-b border-border flex items-center justify-between px-4 bg-surface-variant">
      <div class="flex items-center gap-2">
        <div class="i-carbon-image text-xl text-primary" />
        <span class="font-medium">图标工坊</span>
      </div>

      <div class="flex items-center gap-2">
        <n-button
          secondary
          size="small"
          :type="editorOpen ? 'primary' : 'default'"
          :disabled="showProgressOverlay"
          @click="editorOpen = !editorOpen"
        >
          <template #icon>
            <div class="i-carbon-color-palette" />
          </template>
          SVG 编辑器
        </n-button>
        <n-button
          secondary
          type="error"
          size="small"
          :disabled="isSaving || needsConfirm"
          @click="handleCancel"
        >
          取消 / 关闭
        </n-button>
      </div>
    </div>

    <!-- 主内容区：左侧结果网格 + 右侧固定编辑抽屉 -->
    <div class="flex-1 overflow-hidden p-4">
      <div class="relative h-full">
        <!-- 进度覆盖层 -->
        <transition
          enter-active-class="transition duration-200 ease-out"
          enter-from-class="opacity-0 translate-y-2"
          enter-to-class="opacity-100 translate-y-0"
          leave-active-class="transition duration-150 ease-in"
          leave-from-class="opacity-100 translate-y-0"
          leave-to-class="opacity-0 translate-y-2"
        >
          <SaveProgressOverlay
            v-if="showProgressOverlay"
            :is-saving="isSaving"
            :progress="saveProgress"
            :saving-icon-name="savingIconName"
            :summary="saveSummary"
            :error="saveError"
            @confirm="handleConfirmClose"
          />
        </transition>

        <div class="h-full flex gap-4">
          <!-- 左侧：搜索 + 结果网格 -->
          <div class="flex-1 min-w-0 min-h-0 overflow-hidden icon-popup-scope">
            <IconWorkshop
              :active="true"
              :initial-query="props.initialQuery"
              :initial-style="props.initialStyle"
              :initial-save-path="props.initialSavePath"
              :project-root="props.projectRoot"
              :external-save="true"
              @save="handlePopupSave"
              @selection-change="handleSelectionChange"
              @icon-dblclick="handleIconDblClick"
              @icon-contextmenu="handleIconContextMenu"
            />
          </div>

          <!-- 右侧：固定编辑抽屉 -->
          <transition
            enter-active-class="transition-all duration-200 ease-out"
            enter-from-class="opacity-0 translate-x-4"
            enter-to-class="opacity-100 translate-x-0"
            leave-active-class="transition-all duration-150 ease-in"
            leave-from-class="opacity-100 translate-x-0"
            leave-to-class="opacity-0 translate-x-4"
          >
            <div v-if="editorOpen" class="w-[400px] flex-shrink-0 min-h-0">
              <IconEditorDrawer
                :editor="editor"
                :selected-icons="selectedIcons"
                :initial-save-path="props.initialSavePath"
                :disabled="showProgressOverlay"
                @save="handleEditorSave"
                @close="editorOpen = false"
              />
            </div>
          </transition>
        </div>
      </div>
    </div>

    <!-- 右键上下文菜单 -->
    <IconContextMenu
      :visible="contextMenuVisible"
      :position="contextMenuPosition"
      @open-editor="contextMenuOpenEditor"
      @copy-svg="contextMenuCopySvg"
      @close="closeContextMenu"
    />
  </div>
</template>

<style scoped>
/* 弹窗模式下放大图标预览与网格 - 大尺寸预览优化 */
.icon-popup-scope :deep(.icon-grid) {
  grid-template-columns: repeat(auto-fill, minmax(clamp(100px, 12vw, 140px), 1fr));
  gap: clamp(8px, 1.5vw, 12px);
}

.icon-popup-scope :deep(.icon-card) {
  padding: clamp(8px, 1.5vw, 12px);
  aspect-ratio: 1;
}

.icon-popup-scope :deep(.icon-preview) {
  width: clamp(48px, 8vw, 64px);
  height: clamp(48px, 8vw, 64px);
}

.icon-popup-scope :deep(.font-icon) {
  font-size: clamp(32px, 6vw, 48px);
}

.icon-popup-scope :deep(.skeleton-icon) {
  width: clamp(48px, 8vw, 64px);
  height: clamp(48px, 8vw, 64px);
}

.icon-popup-scope :deep(.icon-name) {
  font-size: clamp(10px, 1vw, 12px);
  margin-top: 4px;
}
</style>
