<template>
  <main class="relative flex min-h-screen w-full items-center justify-center bg-slate-50 px-4 py-16 dark:bg-slate-950">
    <div class="absolute right-3 top-3 flex items-center gap-1">
      <ThemeToggle />
      <button
        type="button"
        aria-label="EN/ES"
        title="EN/ES"
        class="inline-flex h-10 items-center rounded-xl px-2.5 text-sm font-medium text-slate-600 transition-colors hover:bg-slate-200 dark:text-slate-300 dark:hover:bg-slate-800"
        @click="toggleLang"
      >
        <AppIcon name="globe" :size="18" aria-hidden="true" />
        <span class="ml-1 uppercase">{{ ui.locale }}</span>
      </button>
    </div>
    <AppCard class="relative w-full max-w-md !p-6 sm:!p-9" label="sign-in">
      <div class="mb-7 flex items-center gap-3">
        <span class="inline-flex size-12 items-center justify-center rounded-lg bg-indigo-600 text-white" aria-hidden="true">
          <AppIcon name="dashboard" :size="20" />
        </span>
        <div>
          <p class="text-sm font-semibold text-slate-900 dark:text-white">{{ t('app.title') }}</p>
          <p class="mt-1 text-sm text-slate-500 dark:text-slate-400">{{ t('app.workspace') }}</p>
        </div>
      </div>
      <h1 class="text-2xl font-semibold tracking-tight text-slate-950 dark:text-white">{{ t('auth.signIn') }}</h1>
      <p class="mb-6 mt-2 text-sm leading-6 text-slate-600 dark:text-slate-300">{{ copy.description }}</p>
      <div v-if="route.query.next && route.query.next !== '/app'" role="note" class="mb-4 flex items-start gap-2 rounded-lg border border-amber-200 bg-amber-50 p-3 text-sm text-amber-800 dark:border-amber-900 dark:bg-amber-950 dark:text-amber-200">
        <AppIcon name="info" :size="16" class="mt-0.5 shrink-0" aria-hidden="true" />
        {{ t('errors.expired') }}
      </div>
      <AppButton v-if="googleAvailable" variant="secondary" icon="external" href="/api/auth/sign-in/social/google" class="w-full">
        {{ t('auth.signInGoogle') }}
      </AppButton>
      <div v-if="googleAvailable" class="my-4 flex items-center gap-3 text-xs text-slate-500 dark:text-slate-400" aria-hidden="true">
        <span class="h-px flex-1 bg-slate-200 dark:bg-slate-700" />
        {{ t('auth.signInPassword') }}
        <span class="h-px flex-1 bg-slate-200 dark:bg-slate-700" />
      </div>
      <form class="flex flex-col gap-4" @submit.prevent="submit">
        <AppInput v-model="identifier" :label="t('auth.identifier')" type="text" required autocomplete="username" icon="account"  />
        <AppInput v-model="password" :label="t('auth.password')" type="password" required autocomplete="current-password" icon="lock" />
        <p v-if="failed" role="alert" class="flex items-start gap-2 text-sm text-red-700 dark:text-red-300">
          <AppIcon name="alert" :size="16" class="mt-0.5 shrink-0" aria-hidden="true" />
          {{ t('errors.actionFailed') }}
        </p>
        <AppButton type="submit" variant="primary" icon="arrowRight" :loading="busy" class="w-full">
          {{ t('auth.signIn') }}
        </AppButton>
      </form>
      <p class="mt-6 border-t border-slate-200 pt-4 text-xs leading-5 text-slate-600 dark:border-slate-700 dark:text-slate-300">{{ copy.access }}</p>
    </AppCard>
  </main>
</template>

<script setup lang="ts">
import { computed, onMounted, ref } from 'vue'
import { useI18n } from 'vue-i18n'
import { useRoute, useRouter } from 'vue-router'
import { api } from '../api/client'
import { safeNextPath } from '../api/auth'
import { useInvalidateSession } from '../queries/session'
import { useUiStore } from '../stores/ui'
import AppIcon from '../components/icons/AppIcon.vue'
import AppButton from '../components/ui/AppButton.vue'
import AppCard from '../components/ui/AppCard.vue'
import AppInput from '../components/ui/AppInput.vue'
import ThemeToggle from '../components/ui/ThemeToggle.vue'

const { t, locale } = useI18n()
const copy = computed(() => locale.value === 'es' ? {
  description: 'Accede a tu espacio de operaciones con tu cuenta de Cloud.', access: 'Las herramientas disponibles dependen de los permisos de tu cuenta.',
} : {
  description: 'Sign in to your operations workspace with your Cloud account.', access: 'Available tools depend on your account permissions.',
})
const route = useRoute()
const router = useRouter()
const invalidate = useInvalidateSession()
const ui = useUiStore()
const identifier = ref('')
const password = ref('')
const busy = ref(false)
const failed = ref(false)
const googleAvailable = ref(false)

onMounted(async () => {
  try {
    googleAvailable.value = (await api.authProviders()).google === true
  } catch {
    googleAvailable.value = false
  }
})

function toggleLang() {
  ui.toggleLocale()
  locale.value = ui.locale
}

async function submit() {
  if (busy.value) return
  busy.value = true
  failed.value = false
  try {
    // Upstream Better Auth entry points; no hand-rolled token exchange.
    // An @ routes to email sign-in, anything else to username sign-in.
    const id = identifier.value.trim()
    if (id.includes('@')) await api.signInEmail(id, password.value)
    else await api.signInUsername(id, password.value)
    await invalidate()
    await router.replace(safeNextPath(route.query.next))
  } catch {
    failed.value = true
  } finally {
    busy.value = false
  }
}
</script>
