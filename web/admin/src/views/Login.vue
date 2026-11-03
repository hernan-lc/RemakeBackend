<template>
  <main class="flex min-h-screen w-full items-center justify-center bg-slate-100 px-4 py-8 dark:bg-slate-950">
    <div class="absolute right-3 top-3 flex items-center gap-1">
      <ThemeToggle />
      <button
        type="button"
        aria-label="EN/ES"
        title="EN/ES"
        class="inline-flex h-10 items-center rounded-md px-2.5 text-sm font-medium text-slate-600 transition-colors hover:bg-slate-200 dark:text-slate-300 dark:hover:bg-slate-800"
        @click="toggleLang"
      >
        <AppIcon name="globe" :size="18" aria-hidden="true" />
        <span class="ml-1 uppercase">{{ ui.locale }}</span>
      </button>
    </div>
    <AppCard class="w-full max-w-md" label="sign-in">
      <div class="mb-5 flex items-center gap-3">
        <span class="inline-flex items-center justify-center rounded-md bg-slate-900 p-2 text-white dark:bg-slate-100 dark:text-slate-900" aria-hidden="true">
          <AppIcon name="dashboard" :size="20" />
        </span>
        <div>
          <h1 class="text-xl font-bold text-slate-900 dark:text-white">{{ t('app.title') }}</h1>
          <p class="text-sm text-slate-600 dark:text-slate-400">{{ t('auth.signIn') }}</p>
        </div>
      </div>
      <div v-if="route.query.next && route.query.next !== '/app'" role="note" class="mb-4 flex items-start gap-2 rounded-lg border border-amber-200 bg-amber-50 p-3 text-sm text-amber-800 dark:border-amber-900 dark:bg-amber-950 dark:text-amber-200">
        <AppIcon name="info" :size="16" class="mt-0.5 shrink-0" aria-hidden="true" />
        {{ t('errors.expired') }}
      </div>
      <AppButton variant="primary" icon="external" href="/api/auth/sign-in/social/google" class="w-full">
        {{ t('auth.signInGoogle') }}
      </AppButton>
      <div class="my-4 flex items-center gap-3 text-xs text-slate-500 dark:text-slate-400" aria-hidden="true">
        <span class="h-px flex-1 bg-slate-200 dark:bg-slate-700" />
        {{ t('auth.signInEmail') }}
        <span class="h-px flex-1 bg-slate-200 dark:bg-slate-700" />
      </div>
      <form class="flex flex-col gap-3" @submit.prevent="submit">
        <AppInput v-model="email" :label="t('auth.email')" type="email" required autocomplete="username" icon="account" placeholder="admin@example.com" />
        <AppInput v-model="password" :label="t('auth.password')" type="password" required autocomplete="current-password" icon="lock" />
        <p v-if="failed" role="alert" class="flex items-start gap-2 text-sm text-red-700 dark:text-red-300">
          <AppIcon name="alert" :size="16" class="mt-0.5 shrink-0" aria-hidden="true" />
          {{ t('errors.actionFailed') }}
        </p>
        <AppButton type="submit" variant="primary" icon="arrowRight" :loading="busy" class="w-full">
          {{ t('auth.signIn') }}
        </AppButton>
      </form>
    </AppCard>
  </main>
</template>

<script setup lang="ts">
import { ref } from 'vue'
import { useI18n } from 'vue-i18n'
import { useRoute, useRouter } from 'vue-router'
import { ApiError, api } from '../api/client'
import { safeNextPath } from '../api/auth'
import { useInvalidateSession } from '../queries/session'
import { useUiStore } from '../stores/ui'
import AppIcon from '../components/icons/AppIcon.vue'
import AppButton from '../components/ui/AppButton.vue'
import AppCard from '../components/ui/AppCard.vue'
import AppInput from '../components/ui/AppInput.vue'
import ThemeToggle from '../components/ui/ThemeToggle.vue'

const { t, locale } = useI18n()
const route = useRoute()
const router = useRouter()
const invalidate = useInvalidateSession()
const ui = useUiStore()
const email = ref('')
const password = ref('')
const busy = ref(false)
const failed = ref(false)

function toggleLang() {
  ui.toggleLocale()
  locale.value = ui.locale
}

async function submit() {
  busy.value = true
  failed.value = false
  try {
    // Upstream Better Auth entry point; no hand-rolled token exchange.
    await api.signInEmail(email.value, password.value)
    await invalidate()
    await router.replace(safeNextPath(route.query.next))
  } catch (e) {
    failed.value = e instanceof ApiError
  } finally {
    busy.value = false
  }
}
</script>
