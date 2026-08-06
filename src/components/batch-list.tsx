import { Trans } from '@lingui/react/macro'
import type { Batch } from '@/lib/store'
import { cn } from '@/lib/utils'
import { AlertIcon, CheckIcon, SpinnerIcon } from './icons'
import { Badge } from './ui/badge'
import { Card } from './ui/card'

function StatusBadge({ batch }: { batch: Batch }) {
  switch (batch.status) {
    case 'pending':
      return (
        <Badge variant="muted">
          <Trans>Pending</Trans>
        </Badge>
      )
    case 'running':
      return (
        <Badge variant="default">
          <SpinnerIcon width={12} height={12} />
          <Trans>Repatching</Trans>
        </Badge>
      )
    case 'done':
      return (
        <Badge variant="success">
          <CheckIcon width={12} height={12} />
          <Trans>Done</Trans>
        </Badge>
      )
    case 'error':
      return (
        <Badge variant="destructive">
          <AlertIcon width={12} height={12} />
          <Trans>Corrupted files</Trans>
        </Badge>
      )
  }
}

function BatchItem({ batch }: { batch: Batch }) {
  const pct = batch.patchesFound > 0 ? Math.round((batch.checked / batch.patchesFound) * 100) : 0
  return (
    <Card className="p-4">
      <div className="flex items-start justify-between gap-3">
        <div className="min-w-0">
          <div className="truncate font-medium">{batch.name}</div>
          <div className="truncate text-xs text-muted-foreground" title={batch.path}>
            {batch.path}
          </div>
        </div>
        <StatusBadge batch={batch} />
      </div>

      {batch.status === 'running' && (
        <div className="mt-3">
          <div className="h-1.5 w-full overflow-hidden rounded-full bg-muted">
            <div
              className="h-full rounded-full bg-primary transition-all"
              style={{ width: `${pct}%` }}
            />
          </div>
          <div className="mt-1.5 text-xs text-muted-foreground">
            <Trans>
              Checked {batch.checked} of {batch.patchesFound}
            </Trans>
          </div>
        </div>
      )}

      {(batch.status === 'done' || batch.status === 'error') && (
        <div className="mt-3 flex flex-wrap gap-x-4 gap-y-1 text-xs text-muted-foreground">
          <span>
            <Trans>{batch.patchesFound} checked</Trans>
          </span>
          <span className="text-[var(--success)]">
            <Trans>{batch.updated} updated</Trans>
          </span>
          <span>
            <Trans>{batch.skipped} skipped</Trans>
          </span>
          {batch.corrupted.length > 0 && (
            <span className="text-destructive">
              <Trans>{batch.corrupted.length} corrupted</Trans>
            </span>
          )}
        </div>
      )}

      {batch.corrupted.length > 0 && (
        <ul className="mt-2 space-y-0.5 text-xs text-destructive">
          {batch.corrupted.map((c) => (
            <li key={c} className="truncate">
              {c}
            </li>
          ))}
        </ul>
      )}
    </Card>
  )
}

export function BatchList({ batches }: { batches: Batch[] }) {
  if (batches.length === 0) {
    return (
      <div className="rounded-lg border border-dashed border-border p-8 text-center text-sm text-muted-foreground">
        <Trans>No work yet. Drop a mod folder above to start repatching.</Trans>
      </div>
    )
  }
  return (
    <div className={cn('flex flex-col gap-3')}>
      {batches.map((b) => (
        <BatchItem key={b.id} batch={b} />
      ))}
    </div>
  )
}
