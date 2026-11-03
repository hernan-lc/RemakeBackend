<template>
  <main class="mx-auto flex min-h-screen w-full max-w-md flex-col justify-center gap-3 px-4">
    <h1 class="text-2xl font-bold text-slate-900">403</h1>
    <p role="alert" class="text-slate-700">{{ t('errors.forbidden') }}</p>
    <div class="flex gap-2">
      <RouterLink to="/app" class="rounded bg-slate-900 px-3 py-1.5 text-sm font-medium text-white hover:bg-slate-700">
        {{ t('app.dashboard') }}
      </RouterLink>
      <RouterLink to="/logout" class="rounded border border-slate-300 px-3 py-1.5 text-sm font-medium text-slate-700 hover:bg-slate-50">
        {{ t('app.logout') }}
      </RouterLink>
    </div>
  </main>
</template>

<script setup lang="ts">
import { useI18n } from 'vue-i18n'
import { RouterLink } from 'vue-router'

const { t } = useI18n()
</script>
