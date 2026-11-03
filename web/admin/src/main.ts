import { createApp } from 'vue'
import { createI18n } from 'vue-i18n'
import { createPinia } from 'pinia'
import { VueQueryPlugin, QueryClient } from '@tanstack/vue-query'
import App from './App.vue'
import router from './router'
import { en } from './i18n/en'
import { es } from './i18n/es'
import './style.css'

const i18n = createI18n({
  legacy: false,
  locale: 'en',
  fallbackLocale: 'en',
  messages: { en, es },
})

// Pinia holds client-only UI state (locale, sidebar, toasts). Server state
// (session, admin data) lives in TanStack Query and is never in Pinia.
const queryClient = new QueryClient({
  defaultOptions: {
    queries: {
      retry: (count, error: unknown) => {
        const status = (error as { status?: number })?.status ?? 0
        return status >= 500 && count < 1
      },
    },
  },
})

const app = createApp(App)
app.use(createPinia())
app.use(router)
app.use(i18n)
app.use(VueQueryPlugin, { queryClient })
app.mount('#app')
