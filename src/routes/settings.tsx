import { Trans } from '@lingui/react/macro'
import { useState } from 'react'
import { ACCENTS, type Accent, type Theme, useTheme } from '@/components/theme-provider'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from '@/components/ui/card'
import { activateLocale, defaultLocale, locales } from '@/lib/i18n'
import { useStore } from '@/lib/store'
import { api, pickFolder, pickPatchPath } from '@/lib/tauri'
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
  const [gameDataBusy, setGameDataBusy] = useState(false)
  const [gameDataMessage, setGameDataMessage] = useState<string | null>(null)
  const language = config?.language ?? defaultLocale

  async function changeGameRoot() {
    const path = await pickFolder('Select the Helldivers II install folder')
    if (!path) return
    const valid = await api.setGameRoot(path)
    patchConfig({ gameRootPath: path, gameRootValid: valid, unitCount: 0, resourcesReady: false })
    if (valid) {
      setBusy(true)
      try {
        const count = await api.initGameResources(path)
        patchConfig({ unitCount: count, resourcesReady: true })
      } finally {
        setBusy(false)
      }
    }
  }

  async function changeLanguage(next: string) {
    await api.setLanguage(next)
    await activateLocale(next)
    patchConfig({ language: next })
  }

  async function patchGameData() {
    const path = await pickPatchPath('Select a game-data patch or companion')
    if (!path) return
    setGameDataBusy(true)
    setGameDataMessage(null)
    try {
      const result = await api.patchGameData(path)
      setGameDataMessage(`Patched ${result.main}`)
    } catch (error) {
      setGameDataMessage(String(error))
    } finally {
      setGameDataBusy(false)
    }
  }

  const patchReady = Boolean(config?.gameRootValid && config.resourcesReady && !busy)

  return (
    <div className="flex flex-col gap-6">
      <h1 className="text-xl font-semibold">
        <Trans>Settings</Trans>
      </h1>

      <Card>
        <CardHeader>
          <CardTitle>
            <Trans>Game installation folder</Trans>
          </CardTitle>
          <CardDescription>
            <Trans>
              The Helldivers II install root. The app derives its `data` folder for unit and audio
              resources.
            </Trans>
          </CardDescription>
        </CardHeader>
        <CardContent className="flex flex-col gap-3">
          <div className="flex items-center gap-2">
            <code className="min-w-0 flex-1 truncate rounded-md bg-muted px-3 py-2 text-xs">
              {config?.gameRootPath ?? '—'}
            </code>
            {config?.gameRootPath &&
              (config.gameRootValid ? (
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
            <Button variant="outline" onClick={changeGameRoot} disabled={busy}>
              <Trans>Change…</Trans>
            </Button>
            {busy ? (
              <span className="text-xs text-muted-foreground">
                <Trans>Indexing game data…</Trans>
              </span>
            ) : (
              config?.gameRootValid && (
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
            <Trans>Patch game data</Trans>
          </CardTitle>
          <CardDescription>
            <Trans>
              Select one game-data patch or companion to repatch its group in place. If you use a
              mod manager, patch the mod source through the manager instead.
            </Trans>
            <br />
            <Trans>
              Patch groups keep their original filenames; existing `.stream` and `.gpu_resources`
              companions are preserved.
            </Trans>
          </CardDescription>
        </CardHeader>
        <CardContent className="flex flex-col items-start gap-3">
          <Button variant="outline" onClick={patchGameData} disabled={!patchReady || gameDataBusy}>
            <Trans>Choose game-data patch</Trans>
          </Button>
          {!config?.gameRootValid && (
            <span className="text-xs text-destructive">
              <Trans>Set a valid game install root before patching game data.</Trans>
            </span>
          )}
          {config?.gameRootValid && !config.resourcesReady && (
            <span className="text-xs text-muted-foreground">
              <Trans>Indexing game data…</Trans>
            </span>
          )}
          {gameDataMessage && (
            <span className="break-all text-xs text-muted-foreground">{gameDataMessage}</span>
          )}
        </CardContent>
      </Card>

      <Card>
        <CardHeader>
          <CardTitle>
            <Trans>Language</Trans>
          </CardTitle>
          <CardDescription>
            <Trans>Choose the language used by the app.</Trans>
          </CardDescription>
        </CardHeader>
        <CardContent>
          <select
            value={language}
            onChange={(event) => void changeLanguage(event.target.value)}
            className="h-9 rounded-md border border-border bg-background px-3 text-sm"
          >
            {Object.entries(locales).map(([code, name]) => (
              <option key={code} value={code}>
                {name}
              </option>
            ))}
          </select>
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

      <Card>
        <CardHeader>
          <CardTitle>
            <Trans>Audio patching</Trans>
          </CardTitle>
          <CardDescription>
            <Trans>
              Audio patching is built in. The original audio patcher code came from{' '}
              <a
                href="https://github.com/RaidingForPants/hd2-audio-modder"
                target="_blank"
                rel="noreferrer"
                className="underline"
              >
                hd2-audio-modder
              </a>
              .
            </Trans>
          </CardDescription>
        </CardHeader>
      </Card>
    </div>
  )
}
