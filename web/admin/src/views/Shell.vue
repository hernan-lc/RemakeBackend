<template>
  <div class="min-h-screen bg-slate-50 text-slate-900 dark:bg-slate-950 dark:text-slate-100">
    <a href="#content" class="sr-only focus:not-sr-only focus:absolute focus:left-2 focus:top-2 focus:z-50 focus:rounded focus:bg-white focus:px-3 focus:py-1.5 focus:text-sm">{{ t('app.skipContent') }}</a>
    <div class="flex min-h-screen">
      <!-- Navigation rail: icons only on mobile, labels from md up. -->
      <aside
        :aria-label="t('app.menu')"
        :class="[
          'sticky top-0 flex h-screen shrink-0 flex-col border-r border-slate-200 bg-white transition-[width] dark:border-slate-800 dark:bg-slate-900',
          sidebarOpen ? 'w-16 md:w-64' : 'w-16',
        ]"
      >
        <div class="flex h-14 items-center gap-2 border-b border-slate-200 px-3 dark:border-slate-800">
          <span class="inline-flex shrink-0 items-center justify-center rounded-md bg-slate-900 p-1.5 text-white dark:bg-slate-100 dark:text-slate-900" aria-hidden="true">
            <AppIcon name="dashboard" :size="18" />
          </span>
          <span v-if="sidebarOpen" class="hidden truncate text-base font-bold md:inline">{{ t('app.title') }}</span>
        </div>
        <nav aria-label="primary" class="scroll-thin flex-1 overflow-y-auto p-2">
          <ul class="flex flex-col gap-1">
            <li v-for="link in links" :key="link.to">
              <RouterLink
                :to="link.to"
                :title="link.label"
                :aria-label="link.label"
                class="flex items-center gap-3 rounded-md px-3 py-2.5 text-sm font-medium text-slate-600 transition-colors hover:bg-slate-100 hover:text-slate-900 dark:text-slate-400 dark:hover:bg-slate-800 dark:hover:text-white"
                active-class=""
                exact-active-class="!bg-indigo-50 !text-indigo-800 dark:!bg-indigo-950 dark:!text-indigo-200"
              >
                <AppIcon :name="link.icon" :size="19" class="shrink-0" aria-hidden="true" />
                <span v-if="sidebarOpen" class="hidden truncate md:inline">{{ link.label }}</span>
              </RouterLink>
            </li>
          </ul>
        </nav>
        <div class="border-t border-slate-200 p-2 dark:border-slate-800">
          <button
            type="button"
            :aria-label="t(sidebarOpen ? 'app.collapseNav' : 'app.expandNav')"
            :title="t(sidebarOpen ? 'app.collapseNav' : 'app.expandNav')"
            :aria-pressed="!sidebarOpen"
            class="hidden w-full items-center gap-3 rounded-md px-3 py-2.5 text-sm font-medium text-slate-600 transition-colors hover:bg-slate-100 hover:text-slate-900 md:flex dark:text-slate-400 dark:hover:bg-slate-800 dark:hover:text-white"
            @click="ui.toggleSidebar()"
          >
            <AppIcon name="menu" :size="19" class="shrink-0" aria-hidden="true" />
            <span v-if="sidebarOpen" class="hidden truncate md:inline">{{ t('app.collapseNav') }}</span>
          </button>
        </div>
      </aside>

      <div class="flex min-w-0 flex-1 flex-col">
        <!-- Top bar -->
        <header class="sticky top-0 z-30 flex h-14 items-center gap-1 border-b border-slate-200 bg-white/95 px-2 backdrop-blur sm:gap-2 sm:px-4 dark:border-slate-800 dark:bg-slate-900/95">
          <span class="truncate px-2 text-sm font-semibold sm:hidden">{{ currentLabel }}</span>
          <span v-if="me" class="ml-auto hidden items-center gap-2 text-sm text-slate-600 lg:inline-flex dark:text-slate-400">
            <AppIcon name="account" :size="16" aria-hidden="true" />
            <span class="max-w-64 truncate">{{ me.email }}</span>
            <AppBadge tone="neutral" :title="t('auth.roles')">{{ me.roles.join(', ') || '—' }}</AppBadge>
          </span>
          <div class="ml-auto flex items-center gap-1 lg:ml-0">
            <ThemeToggle />
            <button
              type="button"
              :aria-label="'EN/ES'"
              title="EN/ES"
              class="inline-flex h-10 items-center rounded-md px-2.5 text-sm font-medium text-slate-600 transition-colors hover:bg-slate-100 hover:text-slate-900 dark:text-slate-300 dark:hover:bg-slate-800 dark:hover:text-white"
              @click="toggleLang"
            >
              <AppIcon name="globe" :size="18" aria-hidden="true" />
              <span class="ml-1 hidden uppercase sm:inline">{{ ui.locale }}</span>
            </button>
            <RouterLink
              to="/logout"
              :aria-label="t('app.logout')"
              :title="t('app.logout')"
              class="inline-flex size-10 items-center justify-center rounded-md text-slate-600 transition-colors hover:bg-slate-100 hover:text-slate-900 dark:text-slate-300 dark:hover:bg-slate-800 dark:hover:text-white"
            >
              <AppIcon name="logout" :size="18" aria-hidden="true" />
            </RouterLink>
          </div>
        </header>

        <main id="content" class="mx-auto w-full max-w-7xl flex-1 px-4 py-6 sm:px-8 sm:py-9">
          <StateBlock :loading="isPending" :error="!!error" :retry="true" @retry="refetch">
            <RouterView v-if="me" />
          </StateBlock>
          <div v-if="expired" class="mt-4 rounded-lg border border-amber-200 bg-amber-50 p-4 text-sm text-amber-800 dark:border-amber-900 dark:bg-amber-950 dark:text-amber-200" role="alert">
            {{ t('errors.expired') }}
            <RouterLink to="/login" class="font-medium underline">{{ t('auth.signIn') }}</RouterLink>
          </div>
        </main>
      </div>
    </div>

    <!-- Toasts -->
    <div aria-live="polite" class="fixed bottom-4 right-4 z-50 flex w-[min(24rem,calc(100vw-2rem))] flex-col gap-2">
      <div
        v-for="toast in toasts"
        :key="toast.id"
        :role="toast.kind === 'err' ? 'alert' : 'status'"
        class="flex items-center gap-2 rounded-lg border bg-white px-3 py-2.5 text-sm shadow-lg dark:bg-slate-900"
        :class="toast.kind === 'err' ? 'border-red-200 text-red-800 dark:border-red-900 dark:text-red-200' : 'border-slate-200 text-slate-700 dark:border-slate-700 dark:text-slate-300'"
      >
        <AppIcon :name="toast.kind === 'err' ? 'alert' : 'checkCircle'" :size="17" class="shrink-0" aria-hidden="true" />
        <span class="min-w-0 flex-1 break-words">{{ toast.text }}</span>
        <button type="button" class="shrink-0 underline" @click="ui.dismissToast(toast.id)">{{ t('states.close') }}</button>
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
import AppBadge from '../components/ui/AppBadge.vue'
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

// Session load failure redirects to /login; the guard is UX only.
watch(
  error,
  (e) => {
    if (e?.status === 401) void router.replace({ path: '/login', query: { next: '/app' } })
  },
  { immediate: true },
)
</script>
