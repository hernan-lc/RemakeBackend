<template>
  <main class="mx-auto flex min-h-screen w-full max-w-md flex-col justify-center px-4">
    <h1 class="text-2xl font-bold text-slate-900">{{ t('app.title') }}</h1>
    <p class="mt-1 text-sm text-slate-600">{{ t('auth.signIn') }}</p>
    <div v-if="route.query.next && route.query.next !== '/app'" role="note" class="mt-3 rounded border border-amber-200 bg-amber-50 p-3 text-sm text-amber-800">
      {{ t('errors.expired') }}
    </div>
    <a
      href="/api/auth/sign-in/social/google"
      class="mt-6 rounded bg-slate-900 px-4 py-2 text-center text-sm font-medium text-white hover:bg-slate-700"
    >
      {{ t('auth.signInGoogle') }}
    </a>
    <form class="mt-6 flex flex-col gap-3" @submit.prevent="submit">
      <h2 class="text-sm font-semibold text-slate-800">{{ t('auth.signInEmail') }}</h2>
      <label class="flex flex-col gap-1 text-sm text-slate-700">
        {{ t('auth.email') }}
        <input v-model="email" type="email" required autocomplete="username" class="rounded border border-slate-300 px-3 py-2" />
      </label>
      <label class="flex flex-col gap-1 text-sm text-slate-700">
        {{ t('auth.password') }}
        <input v-model="password" type="password" required autocomplete="current-password" class="rounded border border-slate-300 px-3 py-2" />
      </label>
      <p v-if="failed" role="alert" class="text-sm text-red-700">{{ t('errors.actionFailed') }}</p>
      <button type="submit" :disabled="busy" class="rounded bg-slate-900 px-4 py-2 text-sm font-medium text-white hover:bg-slate-700 disabled:opacity-50">
        {{ t('auth.signIn') }}
      </button>
    </form>
  </main>
</template>

<script setup lang="ts">
import { ref } from 'vue'
import { useI18n } from 'vue-i18n'
import { useRoute, useRouter } from 'vue-router'
import { ApiError, api } from '../api/client'
import { safeNextPath } from '../api/auth'
import { useInvalidateSession } from '../queries/session'

const { t } = useI18n()
const route = useRoute()
const router = useRouter()
const invalidate = useInvalidateSession()
const email = ref('')
const password = ref('')
const busy = ref(false)
const failed = ref(false)

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
