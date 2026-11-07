<template>
  <div class="flex flex-col gap-3 sm:flex-row sm:items-center sm:justify-between">
    <div class="min-w-0">
      <h1 class="break-words text-2xl font-semibold sm:text-3xl tracking-tight text-slate-950 dark:text-white">{{ title }}</h1>
      <p v-if="description" class="mt-1 max-w-2xl text-sm leading-5 text-slate-600 dark:text-slate-300">{{ description }}</p>
    </div>
    <div v-if="$slots.actions" class="flex flex-wrap items-center gap-2 sm:justify-end"><slot name="actions" /></div>
  </div>
</template>

<script setup lang="ts">
import type { IconName } from '../icons/icons'
// Keep the icon prop for backwards compatibility; page icons now live in the navigation.
defineProps<{ title: string; description?: string; icon?: IconName }>()
</script>
