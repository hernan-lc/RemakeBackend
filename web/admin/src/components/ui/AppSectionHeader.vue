<template>
  <div class="flex flex-wrap items-start justify-between gap-3">
    <div class="min-w-0">
      <h2 class="flex items-center gap-2 text-base font-semibold tracking-tight text-slate-900 sm:text-lg dark:text-slate-100"><AppIcon v-if="icon" :name="icon" :size="19" class="shrink-0 text-indigo-600 dark:text-indigo-400" aria-hidden="true" />{{ title }}</h2>
      <p v-if="description" class="mt-1 max-w-2xl text-sm leading-6 text-slate-500 dark:text-slate-400">{{ description }}</p>
    </div>
    <div v-if="$slots.actions" class="flex shrink-0 items-center gap-2"><slot name="actions" /></div>
  </div>
</template>

<script setup lang="ts">
import AppIcon from '../icons/AppIcon.vue'
import type { IconName } from '../icons/icons'

defineProps<{ title: string; description?: string; icon?: IconName }>()
</script>
