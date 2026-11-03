<template>
  <main class="flex min-h-screen w-full items-center justify-center bg-slate-100 px-4 dark:bg-slate-950">
    <div class="flex w-full max-w-md flex-col gap-3">
      <h1 class="text-2xl font-bold text-slate-900 dark:text-white">403</h1>
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

const { t } = useI18n()
</script>
