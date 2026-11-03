<template>
  <div class="flex flex-wrap items-start gap-3">
    <div class="flex min-w-0 items-start gap-3">
      <span v-if="icon" class="mt-0.5 hidden shrink-0 rounded-md bg-slate-900 p-2 text-white sm:inline-flex dark:bg-slate-100 dark:text-slate-900">
        <AppIcon :name="icon" :size="20" aria-hidden="true" />
      </span>
      <div class="min-w-0">
        <h1 class="truncate text-xl font-bold text-slate-900 dark:text-white">{{ title }}</h1>
        <p v-if="description" class="mt-0.5 text-sm text-slate-600 dark:text-slate-400">{{ description }}</p>
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
