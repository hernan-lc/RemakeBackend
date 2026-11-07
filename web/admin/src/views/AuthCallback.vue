<template>
  <main class="flex min-h-screen w-full items-center justify-center bg-slate-50 px-4 py-10 dark:bg-slate-950">
    <div v-if="checking" role="status" class="rounded-xl border border-slate-200 bg-white p-7 text-slate-700 dark:border-slate-700 dark:bg-slate-900 dark:text-slate-300">{{ t('states.loading') }}</div>
    <div v-else role="alert" class="w-full max-w-md rounded-xl border border-red-200 bg-white p-7 text-sm text-red-800 dark:border-red-900 dark:bg-slate-900 dark:text-red-200">
      <h1 class="mb-3 text-xl font-semibold">{{ locale === 'es' ? 'No se pudo iniciar sesión' : 'Unable to sign in' }}</h1>
      {{ t('auth.callbackError') }}
      <RouterLink to="/login" class="mt-2 block font-medium underline">{{ t('auth.signIn') }}</RouterLink>
    </div>
  </main>
</template>

<script setup lang="ts">
import { onMounted, ref } from 'vue'
import { useI18n } from 'vue-i18n'
import { RouterLink, useRoute, useRouter } from 'vue-router'
import { api } from '../api/client'
import { callbackOutcome } from '../api/auth'
import { useInvalidateSession } from '../queries/session'

const { t, locale } = useI18n()
const route = useRoute()
const router = useRouter()
const invalidate = useInvalidateSession()
const checking = ref(true)

// The OAuth exchange happens server-side at /api/auth/callback/*.
// This route only reconciles the outcome: an error param means failure,
// otherwise a fresh /api/v1/me decides between /app and /login.
onMounted(async () => {
  if (callbackOutcome(route.query) === 'error') {
    checking.value = false
    return
  }
  try {
    await api.me()
    await invalidate()
    await router.replace('/app')
  } catch {
    await router.replace('/login')
  }
})
</script>
