import { invoke } from '@tauri-apps/api/core'
import { listen, type UnlistenFn } from '@tauri-apps/api/event'
import { getCurrentWebviewWindow } from '@tauri-apps/api/webviewWindow'
import { open } from '@tauri-apps/plugin-dialog'

export type AppConfig = {
  gameDataPath: string | null
  gameDataValid: boolean
  audioToolPath: string | null
  audioToolValid: boolean
  theme: string | null
  accent: string | null
  unitCount: number
}

export type BatchProgress = {
  id: string
  status: 'running' | 'done' | 'error'
  patchesFound: number
  checked: number
  updated: number
  skipped: number
  audio: number
  corrupted: string[]
}

export type BatchResult = {
  id: string
  path: string
  patchesFound: number
  updated: number
  skipped: number
  audio: number
  corrupted: string[]
}

export const api = {
  getConfig: () => invoke<AppConfig>('get_config'),
  setGamePath: (path: string) => invoke<boolean>('set_game_path', { path }),
  setAudioToolPath: (path: string) => invoke<boolean>('set_audio_tool_path', { path }),
  clearAudioToolPath: () => invoke<void>('clear_audio_tool_path'),
  setTheme: (theme: string) => invoke<void>('set_theme', { theme }),
  setAccent: (accent: string) => invoke<void>('set_accent', { accent }),
  initGameResources: (path: string) => invoke<number>('init_game_resources', { path }),
  processBatch: (id: string, path: string) => invoke<BatchResult>('process_batch', { id, path }),
  takeStartupPaths: () => invoke<string[]>('take_startup_paths'),
}

/** Opens the native folder picker; returns the chosen absolute path or null. */
export async function pickFolder(title: string): Promise<string | null> {
  const selected = await open({ directory: true, multiple: false, title })
  return typeof selected === 'string' ? selected : null
}

/** Opens the native file picker; returns the chosen absolute path or null. */
export async function pickFile(title: string): Promise<string | null> {
  const selected = await open({ directory: false, multiple: false, title })
  return typeof selected === 'string' ? selected : null
}

/** Subscribes to per-batch progress events emitted by the backend. */
export function onBatchProgress(cb: (p: BatchProgress) => void): Promise<UnlistenFn> {
  return listen<BatchProgress>('batch://progress', (e) => cb(e.payload))
}

/** Fires when the cached game data finishes indexing at startup. */
export function onResourcesReady(cb: (unitCount: number) => void): Promise<UnlistenFn> {
  return listen<number>('resources://ready', (e) => cb(e.payload))
}

/** Native OS file/folder drops onto the window (absolute paths). */
export function onFileDrop(cb: (paths: string[]) => void): Promise<UnlistenFn> {
  return getCurrentWebviewWindow().onDragDropEvent((event) => {
    if (event.payload.type === 'drop') cb(event.payload.paths)
  })
}

/** Patch folders forwarded from a second double-click/drag-drop launch onto the exe. */
export function onCliOpenPaths(cb: (paths: string[]) => void): Promise<UnlistenFn> {
  return listen<string[]>('cli://open-paths', (e) => cb(e.payload))
}
