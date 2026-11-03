import { defineStore } from 'pinia'
import type { Locale } from '../i18n'

// Client-only UI state. Nothing here is auth or server data: the session,
// review lists, and audit events live in TanStack Query keyed by endpoint.
export const useUiStore = defineStore('ui', {
  state: () => ({
    locale: 'en' as Locale,
    sidebarOpen: true,
    toasts: [] as { id: number; text: string; kind: 'ok' | 'err' }[],
  }),
  actions: {
    toggleLocale() {
      this.locale = this.locale === 'en' ? 'es' : 'en'
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
