import { invoke } from '@tauri-apps/api/core'
import { listen } from '@tauri-apps/api/event'
import { ref } from 'vue'

const SKIPPED_VERSIONS_KEY = 'sanshu_skipped_versions'

interface UpdateInfo {
  available: boolean
  current_version: string
  latest_version: string
  release_notes: string
  release_url: string
  download_url: string
}

interface UpdateProgress {
  downloaded: number
  total: number
  percentage: number
}

const updateInfo = ref<UpdateInfo | null>(null)
const isChecking = ref(false)
const isUpdating = ref(false)
const downloadProgress = ref(0)
const error = ref<string | null>(null)

function loadSkippedVersions(): Set<string> {
  try {
    const raw = localStorage.getItem(SKIPPED_VERSIONS_KEY)
    return raw ? new Set(JSON.parse(raw) as string[]) : new Set()
  }
  catch {
    return new Set()
  }
}

function saveSkippedVersions(versions: Set<string>) {
  localStorage.setItem(SKIPPED_VERSIONS_KEY, JSON.stringify([...versions]))
}

export function useVersionCheck() {
  async function checkForUpdate(ignoreSkipped = false) {
    error.value = null
    isChecking.value = true
    updateInfo.value = null

    try {
      const info = await invoke<UpdateInfo>('check_for_updates')

      if (info.available) {
        const skipped = loadSkippedVersions()
        if (!ignoreSkipped && skipped.has(info.latest_version)) {
          return
        }
        updateInfo.value = info
      }
      else {
        updateInfo.value = info
      }
    }
    catch (e) {
      error.value = String(e)
    }
    finally {
      isChecking.value = false
    }
  }

  async function downloadAndApply() {
    if (!updateInfo.value?.download_url)
      return

    isUpdating.value = true
    downloadProgress.value = 0
    error.value = null

    const unlisten = await listen<UpdateProgress>('update-progress', (event) => {
      downloadProgress.value = Math.round(event.payload.percentage)
    })

    try {
      await invoke<string>('download_and_apply_update', {
        downloadUrl: updateInfo.value.download_url,
      })
    }
    catch (e) {
      error.value = String(e)
    }
    finally {
      unlisten()
      isUpdating.value = false
    }
  }

  function skipVersion() {
    if (!updateInfo.value)
      return
    const skipped = loadSkippedVersions()
    skipped.add(updateInfo.value.latest_version)
    saveSkippedVersions(skipped)
    updateInfo.value = null
  }

  return {
    updateInfo,
    isChecking,
    isUpdating,
    downloadProgress,
    error,
    checkForUpdate,
    downloadAndApply,
    skipVersion,
  }
}
