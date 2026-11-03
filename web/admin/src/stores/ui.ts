import { defineStore } from 'pinia'
import type { Locale } from '../i18n'

export type Theme = 'light' | 'dark'

const THEME_KEY = 'tiktools-admin-theme'
const LOCALE_KEY = 'tiktools-admin-locale'

function systemTheme(): Theme {
  if (typeof window !== 'undefined' && window.matchMedia?.('(prefers-color-scheme: dark)').matches) {
    return 'dark'
  }
  return 'light'
}

function storedTheme(): Theme | null {
  try {
    const v = localStorage.getItem(THEME_KEY)
    return v === 'light' || v === 'dark' ? v : null
  } catch {
    return null
  }
}

function storedLocale(): Locale | null {
  try {
    const v = localStorage.getItem(LOCALE_KEY)
    return v === 'en' || v === 'es' ? v : null
  } catch {
    return null
  }
}

function applyTheme(theme: Theme) {
  document.documentElement.classList.toggle('dark', theme === 'dark')
  document.documentElement.style.colorScheme = theme
}

// Client-only UI state. Nothing here is auth or server data: the session,
// review lists, and audit events live in TanStack Query keyed by endpoint.
// Theme/locale are pure UI preferences persisted in localStorage — never
// credentials or tokens (the API client uses cookie sessions only).
export const useUiStore = defineStore('ui', {
  state: () => ({
    locale: (storedLocale() ?? 'en') as Locale,
    theme: (storedTheme() ?? systemTheme()) as Theme,
    sidebarOpen: true,
    toasts: [] as { id: number; text: string; kind: 'ok' | 'err' }[],
  }),
  actions: {
    initTheme() {
      applyTheme(this.theme)
      document.documentElement.lang = this.locale
    },
    setTheme(theme: Theme) {
      this.theme = theme
      try {
        localStorage.setItem(THEME_KEY, theme)
      } catch {
        // Private-mode storage failure: theme still applies for the session.
      }
      applyTheme(theme)
    },
    toggleTheme() {
      this.setTheme(this.theme === 'dark' ? 'light' : 'dark')
    },
    toggleLocale() {
      this.locale = this.locale === 'en' ? 'es' : 'en'
      try {
        localStorage.setItem(LOCALE_KEY, this.locale)
      } catch {
        // Non-fatal: locale still applies for the session.
      }
      document.documentElement.lang = this.locale
    },
    toggleSidebar() {
      this.sidebarOpen = !this.sidebarOpen
    },
    toast(text: string, kind: 'ok' | 'err' = 'ok') {
      const id = Date.now() + Math.random()
      this.toasts.push({ id, text, kind })
      setTimeout(() => {
        this.toasts = this.toasts.filter((t) => t.id !== id)
      }, 6000)
    },
    dismissToast(id: number) {
      this.toasts = this.toasts.filter((t) => t.id !== id)
    },
  },
})
