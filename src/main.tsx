import { i18n } from '@lingui/core'
import { I18nProvider } from '@lingui/react'
import { RouterProvider } from '@tanstack/react-router'
import { useState } from 'react'
import { createRoot } from 'react-dom/client'
import { SetupWizard } from './components/setup-wizard'
import { type Accent, applyTheme, type Theme, ThemeProvider } from './components/theme-provider'
import { activateLocale, defaultLocale } from './lib/i18n'
import { useStore } from './lib/store'
import { type AppConfig, api, onBatchProgress, onResourcesReady } from './lib/tauri'
import { router } from './router'
import './styles.css'

async function boot() {
  let theme: Theme = 'system'
  let accent: Accent = 'amber'
  let config: AppConfig | null = null
  try {
    config = await api.getConfig()
    theme = (config.theme as Theme) ?? 'system'
    accent = (config.accent as Accent) ?? 'amber'
    useStore.getState().setConfig(config)
  } catch {
    // Running outside Tauri (e.g. plain `vite`): fall back to defaults.
  }
  await activateLocale(config?.language ?? defaultLocale)
  applyTheme(theme, accent)

  void onBatchProgress((p) => useStore.getState().applyProgress(p))
  void onResourcesReady((count) =>
    useStore.getState().patchConfig({ unitCount: count, gameRootValid: true, resourcesReady: true })
  )
  // Preloading can finish between the first config read and listener setup;
  // refresh once so the controls cannot remain disabled after a missed event.
  void api
    .getConfig()
    .then((next) => useStore.getState().setConfig(next))
    .catch(() => undefined)

  const root = document.getElementById('root')
  if (!root) throw new Error('missing #root')
  createRoot(root).render(
    <I18nProvider i18n={i18n}>
      <ThemeProvider initialTheme={theme} initialAccent={accent}>
        <App initialConfig={config} />
      </ThemeProvider>
    </I18nProvider>
  )
}

function App({ initialConfig }: { initialConfig: AppConfig | null }) {
  const [setupComplete, setSetupComplete] = useState(
    initialConfig === null || Boolean(initialConfig.gameRootValid && initialConfig.language)
  )

  if (!setupComplete && initialConfig) {
    return <SetupWizard config={initialConfig} onComplete={() => setSetupComplete(true)} />
  }
  return <RouterProvider router={router} />
}

void boot()
