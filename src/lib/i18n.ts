import { i18n } from '@lingui/core'

export const locales = { en: 'English' } as const
export type Locale = keyof typeof locales
export const defaultLocale: Locale = 'en'

const catalogs = import.meta.glob<{ messages: Record<string, string> }>('../locales/*/messages.po')

/**
 * Activates a locale, loading its catalog from the `.po` file directly —
 * `@lingui/vite-plugin` compiles `.po` imports on the fly, so no
 * `lingui compile` step or committed catalog output is needed. Where a
 * message is missing, Lingui falls back to the source text baked into the
 * `<Trans>` / `t` macros, so the UI always renders.
 */
export async function activateLocale(locale: string): Promise<void> {
  const selected = locale in locales ? (locale as Locale) : defaultLocale
  const load = catalogs[`../locales/${selected}/messages.po`]
  const { messages } = load ? await load() : { messages: {} }
  i18n.load(selected, messages)
  i18n.activate(selected)
}
