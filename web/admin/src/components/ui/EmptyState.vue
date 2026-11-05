<template>
  <div class="flex flex-col items-center justify-center gap-3 rounded-2xl border border-dashed border-slate-200 bg-slate-50/70 px-6 py-14 text-center dark:border-slate-800 dark:bg-slate-900/45">
    <span class="flex size-14 items-center justify-center rounded-2xl border border-slate-200 bg-white text-indigo-500 shadow-sm dark:border-slate-700 dark:bg-slate-800 dark:text-indigo-300">
      <AppIcon :name="icon" :size="25" aria-hidden="true" />
    </span>
    <div class="max-w-md">
      <p class="text-sm font-semibold text-slate-800 dark:text-slate-100">{{ title }}</p>
      <p v-if="hint" class="mt-1 text-sm leading-6 text-slate-500 dark:text-slate-400">{{ hint }}</p>
    </div>
    <div v-if="$slots.actions" class="mt-2 flex flex-wrap justify-center gap-2"><slot name="actions" /></div>
  </div>
</template>

<script setup lang="ts">
import AppIcon from '../icons/AppIcon.vue'
import type { IconName } from '../icons/icons'
withDefaults(defineProps<{ icon?: IconName; title: string; hint?: string }>(), { icon: 'inbox', hint: undefined })
</script>
