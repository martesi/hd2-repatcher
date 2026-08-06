import { i18n } from '@lingui/core'
import { messages as enMessages } from '@/locales/en/messages'

export const locales = { en: 'English', fr: 'Français' }
export const defaultLocale = 'en'

const catalogs: Record<string, Record<string, string>> = {
  en: enMessages,
}

/**
 * Activates a locale from its compiled catalog. Catalogs are produced by
 * `bun run lingui:extract && bun run lingui:compile`; where a message is
 * missing, Lingui falls back to the source text baked into the `<Trans>` / `t`
 * macros, so the UI always renders.
 */
export async function activateLocale(locale: string): Promise<void> {
  i18n.load(locale, catalogs[locale] ?? {})
  i18n.activate(locale)
}
