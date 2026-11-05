<template>
  <div class="flex flex-col gap-4 pb-2 sm:flex-row sm:items-center sm:justify-between">
    <div class="flex min-w-0 items-start gap-3 sm:gap-4">
      <span v-if="icon" class="hidden size-12 shrink-0 items-center justify-center rounded-2xl border border-indigo-100 bg-indigo-50 text-indigo-600 sm:flex dark:border-indigo-900/60 dark:bg-indigo-500/10 dark:text-indigo-300">
        <AppIcon :name="icon" :size="22" aria-hidden="true" />
      </span>
      <div class="min-w-0">
        <h1 class="break-words text-2xl font-bold tracking-tight text-slate-950 sm:text-[30px] sm:leading-9 dark:text-white">{{ title }}</h1>
        <p v-if="description" class="mt-1.5 max-w-2xl text-sm leading-6 text-slate-500 dark:text-slate-400">{{ description }}</p>
      </div>
    </div>
    <div v-if="$slots.actions" class="flex flex-wrap items-center gap-2 sm:justify-end"><slot name="actions" /></div>
  </div>
</template>

<script setup lang="ts">
import AppIcon from '../icons/AppIcon.vue'
import type { IconName } from '../icons/icons'
defineProps<{ title: string; description?: string; icon?: IconName }>()
</script>
