import { Trans } from '@lingui/react/macro'
import { useState } from 'react'
import { ACCENTS, type Accent, type Theme, useTheme } from '@/components/theme-provider'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from '@/components/ui/card'
import { useStore } from '@/lib/store'
import { api, pickFolder } from '@/lib/tauri'
import { cn } from '@/lib/utils'

const THEMES: Theme[] = ['light', 'dark', 'system']

const accentColor: Record<Accent, string> = {
  amber: 'oklch(0.77 0.16 70)',
  blue: 'oklch(0.62 0.19 260)',
  violet: 'oklch(0.6 0.22 300)',
  green: 'oklch(0.65 0.17 150)',
  rose: 'oklch(0.64 0.22 15)',
}

export function Settings() {
  const { theme, accent, setTheme, setAccent } = useTheme()
  const config = useStore((s) => s.config)
  const patchConfig = useStore((s) => s.patchConfig)
  const [busy, setBusy] = useState(false)

  async function changeGamePath() {
    const path = await pickFolder('Select the Helldivers II `data` folder')
    if (!path) return
    const valid = await api.setGamePath(path)
    patchConfig({ gameDataPath: path, gameDataValid: valid, unitCount: 0 })
    if (valid) {
      setBusy(true)
      try {
        const count = await api.initGameResources(path)
        patchConfig({ unitCount: count })
      } finally {
        setBusy(false)
      }
    }
  }

  return (
    <div className="flex flex-col gap-6">
      <h1 className="text-xl font-semibold">
        <Trans>Settings</Trans>
      </h1>

      <Card>
        <CardHeader>
          <CardTitle>
            <Trans>Game data folder</Trans>
          </CardTitle>
          <CardDescription>
            <Trans>The Helldivers II `data` folder used as the source of current unit data.</Trans>
          </CardDescription>
        </CardHeader>
        <CardContent className="flex flex-col gap-3">
          <div className="flex items-center gap-2">
            <code className="min-w-0 flex-1 truncate rounded-md bg-muted px-3 py-2 text-xs">
              {config?.gameDataPath ?? '—'}
            </code>
            {config?.gameDataPath &&
              (config.gameDataValid ? (
                <Badge variant="success">
                  <Trans>Valid</Trans>
                </Badge>
              ) : (
                <Badge variant="destructive">
                  <Trans>Invalid</Trans>
                </Badge>
              ))}
          </div>
          <div className="flex items-center gap-3">
            <Button variant="outline" onClick={changeGamePath} disabled={busy}>
              <Trans>Change…</Trans>
            </Button>
            {busy ? (
              <span className="text-xs text-muted-foreground">
                <Trans>Indexing game data…</Trans>
              </span>
            ) : (
              config?.gameDataValid && (
                <span className="text-xs text-muted-foreground">
                  <Trans>{config.unitCount} unit resources indexed</Trans>
                </span>
              )
            )}
          </div>
        </CardContent>
      </Card>

      <Card>
        <CardHeader>
          <CardTitle>
            <Trans>Appearance</Trans>
          </CardTitle>
          <CardDescription>
            <Trans>Choose a theme and accent color.</Trans>
          </CardDescription>
        </CardHeader>
        <CardContent className="flex flex-col gap-5">
          <div className="flex flex-col gap-2">
            <span className="text-sm font-medium">
              <Trans>Theme</Trans>
            </span>
            <div className="inline-flex w-fit rounded-md border border-border p-1">
              {THEMES.map((t) => (
                <button
                  key={t}
                  type="button"
                  onClick={() => setTheme(t)}
                  className={cn(
                    'rounded px-3 py-1 text-sm capitalize transition-colors',
                    theme === t
                      ? 'bg-primary text-primary-foreground'
                      : 'text-muted-foreground hover:text-foreground'
                  )}
                >
                  {t}
                </button>
              ))}
            </div>
          </div>

          <div className="flex flex-col gap-2">
            <span className="text-sm font-medium">
              <Trans>Accent</Trans>
            </span>
            <div className="flex gap-2.5">
              {ACCENTS.map((a) => (
                <button
                  key={a}
                  type="button"
                  aria-label={a}
                  onClick={() => setAccent(a)}
                  className={cn(
                    'h-8 w-8 rounded-full ring-offset-2 ring-offset-background transition',
                    accent === a ? 'ring-2 ring-foreground' : 'hover:scale-110'
                  )}
                  style={{ backgroundColor: accentColor[a] }}
                />
              ))}
            </div>
          </div>
        </CardContent>
      </Card>
    </div>
  )
}
