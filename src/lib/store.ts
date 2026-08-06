import { create } from 'zustand'
import type { AppConfig, BatchProgress } from './tauri'
import { baseName } from './utils'

export type BatchStatus = 'pending' | 'running' | 'done' | 'error'

export type Batch = {
  id: string
  path: string
  name: string
  status: BatchStatus
  patchesFound: number
  checked: number
  updated: number
  skipped: number
  corrupted: string[]
}

type State = {
  config: AppConfig | null
  batches: Batch[]
  setConfig: (config: AppConfig) => void
  patchConfig: (partial: Partial<AppConfig>) => void
  addBatch: (path: string) => Batch
  applyProgress: (p: BatchProgress) => void
}

let counter = 0
function newId(): string {
  counter += 1
  return `batch-${Date.now()}-${counter}`
}

export const useStore = create<State>((set) => ({
  config: null,
  batches: [],
  setConfig: (config) => set({ config }),
  patchConfig: (partial) =>
    set((s) => ({ config: s.config ? { ...s.config, ...partial } : s.config })),
  addBatch: (path) => {
    const batch: Batch = {
      id: newId(),
      path,
      name: baseName(path),
      status: 'pending',
      patchesFound: 0,
      checked: 0,
      updated: 0,
      skipped: 0,
      corrupted: [],
    }
    set((s) => ({ batches: [batch, ...s.batches] }))
    return batch
  },
  applyProgress: (p) =>
    set((s) => ({
      batches: s.batches.map((b) =>
        b.id === p.id
          ? {
              ...b,
              status: p.status,
              patchesFound: p.patchesFound,
              checked: p.checked,
              updated: p.updated,
              skipped: p.skipped,
              corrupted: p.corrupted,
            }
          : b
      ),
    })),
}))
