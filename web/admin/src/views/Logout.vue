<template>
  <main class="mx-auto flex min-h-screen w-full max-w-md flex-col justify-center px-4">
    <div role="status" class="text-slate-600">{{ t('states.loading') }}</div>
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
