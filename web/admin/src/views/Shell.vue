<template>
  <div class="min-h-screen bg-slate-100 text-slate-900">
    <a href="#content" class="sr-only focus:not-sr-only">{{ t('states.next') }}</a>
    <header class="border-b border-slate-200 bg-white">
      <div class="mx-auto flex max-w-6xl items-center gap-4 px-4 py-3">
        <RouterLink to="/app" class="text-lg font-bold">{{ t('app.title') }}</RouterLink>
        <nav aria-label="primary" class="hidden gap-1 md:flex">
          <RouterLink
            v-for="link in links"
            :key="link.to"
            :to="link.to"
            class="rounded px-3 py-1.5 text-sm font-medium text-slate-600 hover:bg-slate-100 hover:text-slate-900"
            active-class="!bg-slate-900 !text-white"
          >
            {{ link.label }}
          </RouterLink>
        </nav>
        <div class="ml-auto flex items-center gap-2">
          <span v-if="me" class="hidden text-sm text-slate-600 sm:inline">{{ me.email }} · {{ me.roles.join(', ') || '—' }}</span>
          <button type="button" class="rounded border border-slate-300 px-2 py-1 text-sm" :aria-label="'EN/ES'" @click="toggleLang">EN/ES</button>
          <RouterLink to="/logout" class="rounded border border-slate-300 px-3 py-1.5 text-sm font-medium hover:bg-slate-50">{{ t('app.logout') }}</RouterLink>
        </div>
      </div>
      <nav aria-label="primary-mobile" class="flex gap-1 overflow-x-auto px-4 pb-2 md:hidden">
        <RouterLink
          v-for="link in links"
          :key="link.to"
          :to="link.to"
          class="whitespace-nowrap rounded px-3 py-1.5 text-sm font-medium text-slate-600 hover:bg-slate-100"
          active-class="!bg-slate-900 !text-white"
        >
          {{ link.label }}
        </RouterLink>
      </nav>
    </header>
    <main id="content" class="mx-auto max-w-6xl px-4 py-6">
      <StateBlock :loading="isPending" :error="!!error" :retry="true" @retry="refetch">
        <RouterView v-if="me" />
      </StateBlock>
      <div v-if="expired" class="mt-4 rounded border border-amber-200 bg-amber-50 p-4 text-sm text-amber-800" role="alert">
        {{ t('errors.expired') }}
        <RouterLink to="/login" class="font-medium underline">{{ t('auth.signIn') }}</RouterLink>
      </div>
    </main>
    <div aria-live="polite" class="fixed bottom-4 right-4 flex flex-col gap-2">
      <div
        v-for="toast in toasts"
        :key="toast.id"
        :role="toast.kind === 'err' ? 'alert' : 'status'"
        class="rounded border bg-white px-3 py-2 text-sm shadow"
        :class="toast.kind === 'err' ? 'border-red-200 text-red-800' : 'border-slate-200 text-slate-700'"
      >
        {{ toast.text }}
        <button type="button" class="ml-2 underline" @click="ui.dismissToast(toast.id)">{{ t('states.close') }}</button>
      </div>
    </div>
  </div>
</template>

<script setup lang="ts">
import { computed, watch } from 'vue'
import { useI18n } from 'vue-i18n'
import { RouterLink, RouterView, useRouter } from 'vue-router'
import { useUiStore } from '../stores/ui'
import { useSession } from '../queries/session'
import StateBlock from '../components/StateBlock.vue'

const { t, locale } = useI18n()
const router = useRouter()
const ui = useUiStore()
const { data: me, isPending, error, refetch } = useSession()

const expired = computed(() => error.value?.status === 401)
const toasts = computed(() => ui.toasts)

function toggleLang() {
  ui.toggleLocale()
  locale.value = ui.locale
}

const links = computed(() => [
  { to: '/app', label: t('app.dashboard') },
  { to: '/app/publishers', label: t('app.publishers') },
  { to: '/app/moderation', label: t('app.moderation') },
  { to: '/app/audit', label: t('app.audit') },
  { to: '/app/portal', label: t('app.portal') },
  { to: '/app/account', label: t('app.account') },
])

// Session load failure redirects to /login; the guard is UX only.
watch(
  error,
  (e) => {
    if (e?.status === 401) void router.replace({ path: '/login', query: { next: '/app' } })
  },
  { immediate: true },
)
</script>
