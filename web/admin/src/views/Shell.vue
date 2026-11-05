<template>
  <div class="min-h-screen bg-[#f6f8fc] text-slate-900 dark:bg-[#090f1e] dark:text-slate-100">
    <a href="#content" class="sr-only focus:not-sr-only focus:fixed focus:left-4 focus:top-4 focus:z-50 focus:rounded-lg focus:bg-white focus:px-4 focus:py-2 focus:text-sm">{{ t('app.skipContent') }}</a>
    <div class="flex min-h-screen">
      <!-- Desktop workspace navigation. On mobile, the full content width is preserved. -->
      <aside :aria-label="t('app.menu')"
        :class="['sticky top-0 hidden h-screen shrink-0 flex-col border-r border-slate-200/80 bg-white/90 transition-[width] duration-200 lg:flex dark:border-slate-800 dark:bg-slate-900/80', sidebarOpen ? 'w-56' : 'w-[68px]']">
        <div class="flex h-16 items-center gap-3 border-b border-slate-100 px-4 dark:border-slate-800">
          <span class="flex size-9 shrink-0 items-center justify-center rounded-lg bg-indigo-600 text-white shadow-lg shadow-indigo-600/20" aria-hidden="true">
            <AppIcon name="dashboard" :size="21" />
          </span>
          <div v-if="sidebarOpen" class="min-w-0">
            <p class="truncate text-sm font-bold tracking-tight">{{ t('app.title') }}</p>
          </div>
        </div>
        <nav class="scroll-thin flex-1 overflow-y-auto px-2.5 py-4" :aria-label="t('app.navigation')">
          <ul class="space-y-0.5">
            <li v-for="link in links" :key="link.to">
              <RouterLink :to="link.to" :title="link.label" :aria-label="link.label"
                class="group flex min-h-10 items-center gap-3 rounded-lg px-3 text-sm font-medium text-slate-600 transition-colors hover:bg-slate-100 hover:text-slate-900 dark:text-slate-400 dark:hover:bg-slate-800 dark:hover:text-white"
                exact-active-class="!bg-indigo-50 !text-indigo-700 dark:!bg-indigo-500/15 dark:!text-indigo-300">
                <AppIcon :name="link.icon" :size="19" class="shrink-0" aria-hidden="true" />
                <span v-if="sidebarOpen" class="truncate">{{ link.label }}</span>
              </RouterLink>
            </li>
          </ul>
        </nav>
        <div class="space-y-2 border-t border-slate-100 p-3 dark:border-slate-800">
          <RouterLink v-if="me && sidebarOpen" to="/app/account" class="flex items-center gap-3 rounded-lg bg-slate-50 px-2 py-2 transition-colors hover:bg-slate-100 dark:bg-slate-800/70 dark:hover:bg-slate-800">
            <span class="flex size-9 shrink-0 items-center justify-center rounded-full bg-indigo-100 font-bold text-indigo-700 dark:bg-indigo-900 dark:text-indigo-200">{{ me.name?.charAt(0).toUpperCase() || 'A' }}</span>
            <span class="min-w-0 block truncate text-xs font-semibold">{{ me.name }}</span>
          </RouterLink>
          <button type="button" :aria-label="t(sidebarOpen ? 'app.collapseNav' : 'app.expandNav')" :title="t(sidebarOpen ? 'app.collapseNav' : 'app.expandNav')"
            class="flex min-h-10 w-full items-center gap-3 rounded-lg px-3 text-sm text-slate-500 transition-colors hover:bg-slate-100 dark:hover:bg-slate-800"
            @click="ui.toggleSidebar()">
            <AppIcon name="menu" :size="19" aria-hidden="true" /><span v-if="sidebarOpen" class="text-xs">{{ t('app.collapseNav') }}</span>
          </button>
        </div>
      </aside>

      <div class="flex min-w-0 flex-1 flex-col">
        <header class="sticky top-0 z-30 flex h-14 items-center gap-2 border-b border-slate-200/80 bg-white/90 px-4 backdrop-blur-xl sm:px-6 lg:px-8 dark:border-slate-800 dark:bg-slate-900/85">
          <span class="flex size-9 shrink-0 items-center justify-center rounded-xl bg-indigo-600 text-white lg:hidden" aria-hidden="true">
            <AppIcon name="dashboard" :size="19" />
          </span>
          <p class="min-w-0 flex-1 truncate text-sm font-semibold lg:sr-only">{{ currentLabel }}</p>
          <div class="hidden flex-1 lg:block" aria-hidden="true" />
          <div class="flex shrink-0 items-center gap-0.5 sm:gap-1">
            <ThemeToggle />
            <button type="button" :aria-label="'EN/ES'" title="EN/ES"
              class="inline-flex size-10 items-center justify-center gap-1 rounded-xl text-sm font-medium text-slate-600 transition-colors hover:bg-slate-100 dark:text-slate-300 dark:hover:bg-slate-800" @click="toggleLang">
              <AppIcon name="globe" :size="18" aria-hidden="true" /><span class="hidden text-xs font-bold uppercase sm:inline">{{ ui.locale }}</span>
            </button>
          </div>
        </header>

        <main id="content" class="mx-auto w-full max-w-[1320px] flex-1 px-4 pb-28 pt-5 sm:px-6 lg:px-8 lg:pb-12">
          <StateBlock :loading="isPending" :error="!!error" :retry="true" @retry="refetch">
            <RouterView v-if="me" />
          </StateBlock>
          <div v-if="expired" role="alert" class="mt-4 rounded-xl border border-amber-200 bg-amber-50 p-4 text-sm text-amber-800 dark:border-amber-900 dark:bg-amber-950 dark:text-amber-200">
            {{ t('errors.expired') }} <RouterLink to="/login" class="font-semibold underline">{{ t('auth.signIn') }}</RouterLink>
          </div>
        </main>
      </div>
    </div>

    <!-- Mobile-first bottom navigation: no persistent rail stealing viewport width. -->
    <nav :aria-label="t('app.navigation')" class="mobile-tabbar fixed inset-x-0 bottom-0 z-40 border-t border-slate-200/80 bg-white/95 px-1 pt-2 shadow-[0_-8px_30px_-20px_rgba(15,23,42,0.25)] backdrop-blur-xl lg:hidden dark:border-slate-800 dark:bg-slate-900/95">
      <ul class="grid grid-cols-6">
        <li v-for="link in links" :key="link.to" class="min-w-0">
          <RouterLink :to="link.to" :aria-label="link.label" :title="link.label"
            class="flex min-h-12 flex-col items-center justify-center gap-1 rounded-xl px-0.5 py-1 text-slate-500 transition-colors hover:bg-slate-100 dark:text-slate-400 dark:hover:bg-slate-800"
            exact-active-class="!bg-indigo-50 !text-indigo-700 dark:!bg-indigo-500/15 dark:!text-indigo-300">
            <AppIcon :name="link.icon" :size="20" aria-hidden="true" />
            <span class="block w-full truncate text-center text-[9px] font-semibold leading-3 sm:text-[11px]">{{ link.label }}</span>
          </RouterLink>
        </li>
      </ul>
    </nav>

    <div aria-live="polite" class="fixed bottom-24 right-4 z-50 flex w-[min(24rem,calc(100vw-2rem))] flex-col gap-2 lg:bottom-4">
      <div v-for="toast in toasts" :key="toast.id" :role="toast.kind === 'err' ? 'alert' : 'status'"
        class="flex items-center gap-2 rounded-xl border bg-white px-3 py-3 text-sm shadow-xl dark:bg-slate-900"
        :class="toast.kind === 'err' ? 'border-red-200 text-red-800 dark:border-red-900 dark:text-red-200' : 'border-slate-200 text-slate-700 dark:border-slate-700 dark:text-slate-300'">
        <AppIcon :name="toast.kind === 'err' ? 'alert' : 'checkCircle'" :size="17" class="shrink-0" aria-hidden="true" />
        <span class="min-w-0 flex-1 break-words">{{ toast.text }}</span>
        <button type="button" class="rounded px-1 font-semibold underline" @click="ui.dismissToast(toast.id)">{{ t('states.close') }}</button>
      </div>
    </div>
  </div>
</template>

<script setup lang="ts">
import { computed, watch } from 'vue'
import { useI18n } from 'vue-i18n'
import { RouterLink, RouterView, useRoute, useRouter } from 'vue-router'
import { useUiStore } from '../stores/ui'
import { useSession } from '../queries/session'
import StateBlock from '../components/StateBlock.vue'
import AppIcon from '../components/icons/AppIcon.vue'
import type { IconName } from '../components/icons/icons'
import ThemeToggle from '../components/ui/ThemeToggle.vue'

const { t, locale } = useI18n()
const route = useRoute()
const router = useRouter()
const ui = useUiStore()
const { data: me, isPending, error, refetch } = useSession()

const expired = computed(() => error.value?.status === 401)
const toasts = computed(() => ui.toasts)
const sidebarOpen = computed(() => ui.sidebarOpen)

function toggleLang() {
  ui.toggleLocale()
  locale.value = ui.locale
}

const links = computed<{ to: string; label: string; icon: IconName }[]>(() => [
  { to: '/app', label: t('app.dashboard'), icon: 'dashboard' },
  { to: '/app/publishers', label: t('app.publishers'), icon: 'publishers' },
  { to: '/app/moderation', label: t('app.moderation'), icon: 'moderation' },
  { to: '/app/audit', label: t('app.audit'), icon: 'audit' },
  { to: '/app/portal', label: t('app.portal'), icon: 'portal' },
  { to: '/app/account', label: t('app.account'), icon: 'account' },
])

const currentLabel = computed(() => {
  const match = [...links.value].reverse().find((l) => route.path === l.to || route.path.startsWith(`${l.to}/`))
  return match?.label ?? t('app.title')
})

watch(
  error,
  (e) => {
    if (e?.status === 401) void router.replace({ path: '/login', query: { next: '/app' } })
  },
  { immediate: true },
)
</script>
