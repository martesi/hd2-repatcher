import { createContext, type ReactNode, useContext, useEffect, useState } from 'react'
import { api } from '@/lib/tauri'

export type Theme = 'light' | 'dark' | 'system'
export const ACCENTS = ['amber', 'blue', 'violet', 'green', 'rose'] as const
export type Accent = (typeof ACCENTS)[number]

type ThemeCtx = {
  theme: Theme
  accent: Accent
  setTheme: (t: Theme) => void
  setAccent: (a: Accent) => void
}

const Ctx = createContext<ThemeCtx | null>(null)

/** Reflects theme + accent onto <html> so the CSS variables in styles.css apply. */
export function applyTheme(theme: Theme, accent: Accent): void {
  const root = document.documentElement
  if (theme === 'system') root.removeAttribute('data-theme')
  else root.setAttribute('data-theme', theme)
  root.setAttribute('data-accent', accent)
}

export function ThemeProvider({
  children,
  initialTheme,
  initialAccent,
}: {
  children: ReactNode
  initialTheme: Theme
  initialAccent: Accent
}) {
  const [theme, setThemeState] = useState<Theme>(initialTheme)
  const [accent, setAccentState] = useState<Accent>(initialAccent)

  useEffect(() => {
    applyTheme(theme, accent)
  }, [theme, accent])

  const setTheme = (t: Theme) => {
    setThemeState(t)
    void api.setTheme(t)
  }
  const setAccent = (a: Accent) => {
    setAccentState(a)
    void api.setAccent(a)
  }

  return <Ctx.Provider value={{ theme, accent, setTheme, setAccent }}>{children}</Ctx.Provider>
}

export function useTheme(): ThemeCtx {
  const c = useContext(Ctx)
  if (!c) throw new Error('useTheme must be used within ThemeProvider')
  return c
}
