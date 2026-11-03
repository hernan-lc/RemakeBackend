<template>
  <div class="flex flex-col items-center gap-2 rounded-lg border border-dashed border-slate-300 px-6 py-10 text-center dark:border-slate-700">
    <span class="rounded-full bg-slate-100 p-3 text-slate-500 dark:bg-slate-800 dark:text-slate-400">
      <AppIcon :name="icon" :size="22" aria-hidden="true" />
    </span>
    <p class="text-sm font-medium text-slate-700 dark:text-slate-300">{{ title }}</p>
    <p v-if="hint" class="max-w-sm text-sm text-slate-500 dark:text-slate-400">{{ hint }}</p>
    <div v-if="$slots.actions" class="mt-2 flex flex-wrap justify-center gap-2">
      <slot name="actions" />
    </div>
  </div>
</template>

<script setup lang="ts">
import AppIcon from '../icons/AppIcon.vue'
import type { IconName } from '../icons/icons'

withDefaults(defineProps<{ icon?: IconName; title: string; hint?: string }>(), { icon: 'inbox', hint: undefined })
</script>
