<template>
  <div class="flex flex-wrap items-end gap-4 border-b border-slate-200/80 pb-5 dark:border-slate-800">
    <div class="flex min-w-0 items-start gap-4">
      <span v-if="icon" class="mt-0.5 hidden shrink-0 rounded-xl bg-indigo-100 p-3 text-indigo-700 sm:inline-flex dark:bg-indigo-950 dark:text-indigo-300">
        <AppIcon :name="icon" :size="20" aria-hidden="true" />
      </span>
      <div class="min-w-0">
        <h1 class="break-words text-2xl font-bold tracking-tight text-slate-900 sm:text-3xl dark:text-white">{{ title }}</h1>
        <p v-if="description" class="mt-1 max-w-2xl text-sm leading-6 text-slate-600 dark:text-slate-400">{{ description }}</p>
      </div>
    </div>
    <div v-if="$slots.actions" class="ml-auto flex flex-wrap items-center gap-2">
      <slot name="actions" />
    </div>
  </div>
</template>

<script setup lang="ts">
import AppIcon from '../icons/AppIcon.vue'
import type { IconName } from '../icons/icons'

defineProps<{ title: string; description?: string; icon?: IconName }>()
</script>
