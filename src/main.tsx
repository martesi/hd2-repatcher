import { i18n } from '@lingui/core'
import { I18nProvider } from '@lingui/react'
import { RouterProvider } from '@tanstack/react-router'
import { createRoot } from 'react-dom/client'
import { type Accent, applyTheme, type Theme, ThemeProvider } from './components/theme-provider'
import { activateLocale, defaultLocale } from './lib/i18n'
import { useStore } from './lib/store'
import { api, onBatchProgress, onResourcesReady } from './lib/tauri'
import { router } from './router'
import './styles.css'

async function boot() {
  await activateLocale(defaultLocale)

  let theme: Theme = 'system'
  let accent: Accent = 'amber'
  try {
    const config = await api.getConfig()
    theme = (config.theme as Theme) ?? 'system'
    accent = (config.accent as Accent) ?? 'amber'
    useStore.getState().setConfig(config)
  } catch {
    // Running outside Tauri (e.g. plain `vite`): fall back to defaults.
  }
  applyTheme(theme, accent)

  void onBatchProgress((p) => useStore.getState().applyProgress(p))
  void onResourcesReady((count) =>
    useStore.getState().patchConfig({ unitCount: count, gameDataValid: true })
  )

  const root = document.getElementById('root')
  if (!root) throw new Error('missing #root')
  createRoot(root).render(
    <I18nProvider i18n={i18n}>
      <ThemeProvider initialTheme={theme} initialAccent={accent}>
        <RouterProvider router={router} />
      </ThemeProvider>
    </I18nProvider>
  )
}

void boot()
