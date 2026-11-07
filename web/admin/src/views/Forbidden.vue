<template>
  <main class="flex min-h-screen w-full items-center justify-center bg-slate-50 px-4 py-10 dark:bg-slate-950">
    <div class="flex w-full max-w-md flex-col gap-5 rounded-xl border border-slate-200 bg-white p-7 dark:border-slate-700 dark:bg-slate-900">
      <p class="text-xs font-semibold uppercase tracking-wider text-slate-600 dark:text-slate-300">Cloud · 403</p>
      <h1 class="text-2xl font-semibold text-slate-950 dark:text-white">{{ locale === 'es' ? 'Acceso restringido' : 'Access restricted' }}</h1>
      <p role="alert" class="text-slate-700 dark:text-slate-300">{{ t('errors.forbidden') }}</p>
      <div class="flex flex-wrap gap-2">
        <AppButton variant="primary" to="/app">
          {{ t('app.dashboard') }}
        </AppButton>
        <AppButton variant="secondary" to="/logout">
          {{ t('app.logout') }}
        </AppButton>
      </div>
    </div>
  </main>
</template>

<script setup lang="ts">
import { useI18n } from 'vue-i18n'
import AppButton from '../components/ui/AppButton.vue'

const { t, locale } = useI18n()
</script>
