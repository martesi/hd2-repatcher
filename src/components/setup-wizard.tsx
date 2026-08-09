import { Trans } from '@lingui/react/macro'
import { useState } from 'react'
import { activateLocale, locales } from '@/lib/i18n'
import { useStore } from '@/lib/store'
import { type AppConfig, api, pickFolder } from '@/lib/tauri'
import { Button } from './ui/button'
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from './ui/card'

export function SetupWizard({ config, onComplete }: { config: AppConfig; onComplete: () => void }) {
  const [step, setStep] = useState(config.gameRootPath && config.gameRootValid ? 1 : 0)
  const [gameRoot, setGameRoot] = useState(config.gameRootPath ?? '')
  const [language, setLanguage] = useState(config.language ?? 'en')
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)

  async function selectGameRoot() {
    const path = await pickFolder('Select the Helldivers II install folder')
    if (path) {
      setGameRoot(path)
      setError(null)
    }
  }

  async function continueToLanguage() {
    if (!gameRoot) {
      setError('Select your Helldivers II install folder first.')
      return
    }
    setBusy(true)
    setError(null)
    try {
      const valid = await api.setGameRoot(gameRoot)
      if (!valid) {
        setError('That folder does not contain a valid Helldivers II data folder.')
        return
      }
      useStore.getState().patchConfig({
        gameRootPath: gameRoot,
        gameRootValid: true,
        resourcesReady: false,
      })
      setStep(1)
    } catch (reason) {
      setError(String(reason))
    } finally {
      setBusy(false)
    }
  }

  async function finish() {
    setBusy(true)
    setError(null)
    try {
      const unitCount = await api.initGameResources(gameRoot)
      await api.setLanguage(language)
      await activateLocale(language)
      useStore.getState().patchConfig({
        gameRootPath: gameRoot,
        gameRootValid: true,
        language,
        unitCount,
        resourcesReady: true,
      })
      onComplete()
    } catch (reason) {
      setError(String(reason))
    } finally {
      setBusy(false)
    }
  }

  return (
    <main className="flex min-h-screen items-center justify-center p-8">
      <Card className="w-full max-w-lg">
        <CardHeader>
          <div className="mb-2 text-xs font-medium uppercase tracking-wider text-muted-foreground">
            <Trans>Initial setup</Trans>
          </div>
          <CardTitle>
            {step === 0 ? <Trans>Set up HD2 Repatcher</Trans> : <Trans>Choose your language</Trans>}
          </CardTitle>
          <CardDescription>
            {step === 0 ? (
              <Trans>
                Select the Helldivers II install folder. The app will use its `data` folder for unit
                and audio resources.
              </Trans>
            ) : (
              <Trans>Select the language you want to use, then finish setup.</Trans>
            )}
          </CardDescription>
        </CardHeader>
        <CardContent className="flex flex-col gap-4">
          {step === 0 ? (
            <>
              <div className="flex items-center gap-2">
                <code className="min-w-0 flex-1 truncate rounded-md bg-muted px-3 py-2 text-xs">
                  {gameRoot || '—'}
                </code>
                <Button variant="outline" onClick={selectGameRoot} disabled={busy}>
                  <Trans>Choose folder</Trans>
                </Button>
              </div>
              <Button onClick={() => void continueToLanguage()} disabled={busy}>
                <Trans>Next</Trans>
              </Button>
            </>
          ) : (
            <>
              <label className="flex flex-col gap-2 text-sm font-medium">
                <Trans>Language</Trans>
                <select
                  value={language}
                  onChange={(event) => setLanguage(event.target.value)}
                  disabled={busy}
                  className="h-9 rounded-md border border-border bg-background px-3 font-normal"
                >
                  {Object.entries(locales).map(([code, name]) => (
                    <option key={code} value={code}>
                      {name}
                    </option>
                  ))}
                </select>
              </label>
              <div className="flex gap-2">
                <Button variant="outline" onClick={() => setStep(0)} disabled={busy}>
                  <Trans>Back</Trans>
                </Button>
                <Button className="flex-1" onClick={() => void finish()} disabled={busy}>
                  {busy ? <Trans>Setting up…</Trans> : <Trans>Finish setup</Trans>}
                </Button>
              </div>
            </>
          )}
          {error && <p className="text-sm text-destructive">{error}</p>}
        </CardContent>
      </Card>
    </main>
  )
}
