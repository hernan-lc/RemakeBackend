import { beforeEach, describe, expect, it, vi } from 'vitest'
import { createPinia, setActivePinia } from 'pinia'
import { useUiStore } from '../src/stores/ui'

function installMemoryStorage() {
  const store = new Map<string, string>()
  vi.stubGlobal('localStorage', {
    getItem: (k: string) => (store.has(k) ? store.get(k) : null),
    setItem: (k: string, v: string) => void store.set(k, String(v)),
    removeItem: (k: string) => void store.delete(k),
    clear: () => void store.clear(),
  })
}

beforeEach(() => {
  vi.unstubAllGlobals()
  installMemoryStorage()
  document.documentElement.classList.remove('dark')
  document.documentElement.style.colorScheme = ''
  setActivePinia(createPinia())
})

describe('theme preference', () => {
  it('defaults to light and applies it to the document', () => {
    const ui = useUiStore()
    ui.initTheme()
    expect(ui.theme).toBe('light')
    expect(document.documentElement.classList.contains('dark')).toBe(false)
    expect(document.documentElement.style.colorScheme).toBe('light')
  })

  it('toggles theme, updates the document, and persists the choice', () => {
    const ui = useUiStore()
    ui.initTheme()
    ui.toggleTheme()
    expect(ui.theme).toBe('dark')
    expect(document.documentElement.classList.contains('dark')).toBe(true)
    expect(document.documentElement.style.colorScheme).toBe('dark')
    expect(localStorage.getItem('tiktools-admin-theme')).toBe('dark')
    ui.toggleTheme()
    expect(ui.theme).toBe('light')
    expect(document.documentElement.classList.contains('dark')).toBe(false)
  })

  it('restores the persisted theme on init', () => {
    localStorage.setItem('tiktools-admin-theme', 'dark')
    const ui = useUiStore()
    // Fresh store reads storage in state, init applies it.
    expect(ui.theme).toBe('dark')
    ui.initTheme()
    expect(document.documentElement.classList.contains('dark')).toBe(true)
  })

  it('persists the locale choice', () => {
    const ui = useUiStore()
    ui.toggleLocale()
    expect(ui.locale).toBe('es')
    expect(localStorage.getItem('tiktools-admin-locale')).toBe('es')
    expect(document.documentElement.lang).toBe('es')
  })
})
