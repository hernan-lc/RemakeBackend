<template>
  <main class="mx-auto flex min-h-screen w-full max-w-md flex-col justify-center px-4">
    <div v-if="checking" role="status" class="text-slate-600">{{ t('states.loading') }}</div>
    <div v-else role="alert" class="rounded border border-red-200 bg-red-50 p-4 text-sm text-red-800">
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

const { t } = useI18n()
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
