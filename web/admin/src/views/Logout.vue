<template>
  <main class="flex min-h-screen w-full items-center justify-center bg-slate-50 px-4 py-10 dark:bg-slate-950">
    <div role="status" class="w-full max-w-md rounded-xl border border-slate-200 bg-white p-7 text-slate-700 dark:border-slate-700 dark:bg-slate-900 dark:text-slate-300">{{ t('states.loading') }}</div>
  </main>
</template>

<script setup lang="ts">
import { onMounted } from 'vue'
import { useI18n } from 'vue-i18n'
import { useRouter } from 'vue-router'
import { useQueryClient } from '@tanstack/vue-query'
import { api } from '../api/client'
import { ME_KEY } from '../queries/session'

const { t } = useI18n()
const router = useRouter()
const client = useQueryClient()

// Sign out through the Better Auth endpoint (CSRF-checked server-side),
// then invalidate cached user data and redirect. Never persisted access
// tokens anywhere, so there is nothing else to clear.
onMounted(async () => {
  try {
    await api.signOut()
  } catch {
    // Revocation is best-effort: still drop local cached data below.
  } finally {
    client.setQueryData(ME_KEY, undefined)
    await client.invalidateQueries({ queryKey: ME_KEY })
    await router.replace('/login')
  }
})
</script>
