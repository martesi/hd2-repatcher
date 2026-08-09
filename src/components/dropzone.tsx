import { Trans } from '@lingui/react/macro'
import { getCurrentWebviewWindow } from '@tauri-apps/api/webviewWindow'
import { useEffect, useState } from 'react'
import { pickFolder, pickPatchPath } from '@/lib/tauri'
import { cn } from '@/lib/utils'
import { FolderPlusIcon } from './icons'
import { Button } from './ui/button'

export function Dropzone({
  onAdd,
  disabled,
}: {
  onAdd: (path: string) => void
  disabled?: boolean
}) {
  const [dragging, setDragging] = useState(false)

  useEffect(() => {
    let unlisten: (() => void) | undefined
    getCurrentWebviewWindow()
      .onDragDropEvent((event) => {
        const t = event.payload.type
        if (t === 'enter' || t === 'over') setDragging(true)
        else if (t === 'leave') setDragging(false)
        else if (t === 'drop') {
          setDragging(false)
          if (!disabled) for (const p of event.payload.paths) onAdd(p)
        }
      })
      .then((fn) => {
        unlisten = fn
      })
    return () => unlisten?.()
  }, [onAdd, disabled])

  async function addFolderViaPicker() {
    const path = await pickFolder('Select a mod folder to repatch')
    if (path) onAdd(path)
  }

  async function addPatchViaPicker() {
    const path = await pickPatchPath('Select a patch file or companion to repatch')
    if (path) onAdd(path)
  }

  return (
    <div
      className={cn(
        'flex flex-col items-center justify-center gap-3 rounded-lg border-2 border-dashed p-10 text-center transition-colors',
        dragging ? 'border-primary bg-primary/5' : 'border-border',
        disabled && 'opacity-60'
      )}
    >
      <FolderPlusIcon className="text-muted-foreground" width={30} height={30} />
      <div className="text-sm text-muted-foreground">
        <Trans>Drag a mod folder or patch file here, or choose one</Trans>
      </div>
      <div className="flex gap-2">
        <Button onClick={addFolderViaPicker} disabled={disabled}>
          <Trans>Add folder</Trans>
        </Button>
        <Button variant="outline" onClick={addPatchViaPicker} disabled={disabled}>
          <Trans>Add patch</Trans>
        </Button>
      </div>
    </div>
  )
}
