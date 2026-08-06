import { Trans } from '@lingui/react/macro'
import { Link } from '@tanstack/react-router'
import { useEffect } from 'react'
import { BatchList } from '@/components/batch-list'
import { Dropzone } from '@/components/dropzone'
import { AlertIcon } from '@/components/icons'
import { useStore } from '@/lib/store'
import { api, onCliOpenPaths } from '@/lib/tauri'

export function Home() {
  const batches = useStore((s) => s.batches)
  const addBatch = useStore((s) => s.addBatch)
  const config = useStore((s) => s.config)
  const valid = config?.gameDataValid ?? false

  function handleAdd(path: string) {
    const batch = addBatch(path)
    api.processBatch(batch.id, path).catch((err) => {
      useStore.getState().applyProgress({
        id: batch.id,
        status: 'error',
        patchesFound: 0,
        checked: 0,
        updated: 0,
        skipped: 0,
        corrupted: [String(err)],
      })
    })
  }

  // Patch folders dropped onto the exe: this launch's own argv (pulled once,
  // since the backend can't reliably push an event before this mounts), plus
  // any forwarded from a later double-click/drag-drop while we're running.
  // biome-ignore lint/correctness/useExhaustiveDependencies: run once on mount
  useEffect(() => {
    api.takeStartupPaths().then((paths) => {
      for (const p of paths) handleAdd(p)
    })
    let unlisten: (() => void) | undefined
    onCliOpenPaths((paths) => {
      for (const p of paths) handleAdd(p)
    }).then((fn) => {
      unlisten = fn
    })
    return () => unlisten?.()
  }, [])

  return (
    <div className="flex flex-col gap-6">
      <div>
        <h1 className="text-xl font-semibold">
          <Trans>Repatch mods</Trans>
        </h1>
        <p className="text-sm text-muted-foreground">
          <Trans>
            Drop a mod folder to update its unit resources against the current game data.
          </Trans>
        </p>
      </div>

      {!valid && (
        <div className="flex items-start gap-3 rounded-lg border border-destructive/40 bg-destructive/10 p-4 text-sm">
          <AlertIcon className="mt-0.5 shrink-0 text-destructive" />
          <div>
            <Trans>
              No valid Helldivers II game data folder is set. Set it in{' '}
              <Link to="/settings" className="font-medium underline">
                Settings
              </Link>{' '}
              before repatching.
            </Trans>
          </div>
        </div>
      )}

      <Dropzone onAdd={handleAdd} disabled={!valid} />

      <div>
        <h2 className="mb-3 text-sm font-medium text-muted-foreground">
          <Trans>Work</Trans>
        </h2>
        <BatchList batches={batches} />
      </div>
    </div>
  )
}
