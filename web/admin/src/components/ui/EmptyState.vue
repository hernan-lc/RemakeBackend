<template>
  <div class="flex flex-col items-center justify-center gap-2 rounded-xl border border-dashed border-slate-200 px-5 py-9 text-center dark:border-slate-800">
    <AppIcon :name="icon" :size="21" class="text-slate-400 dark:text-slate-500" aria-hidden="true" />
    <p class="text-sm font-medium text-slate-700 dark:text-slate-200">{{ title }}</p>
    <p v-if="hint" class="max-w-md text-xs leading-5 text-slate-500 dark:text-slate-400">{{ hint }}</p>
    <div v-if="$slots.actions" class="mt-2 flex flex-wrap justify-center gap-2"><slot name="actions" /></div>
  </div>
</template>

<script setup lang="ts">
import AppIcon from '../icons/AppIcon.vue'
import type { IconName } from '../icons/icons'
withDefaults(defineProps<{ icon?: IconName; title: string; hint?: string }>(), { icon: 'inbox', hint: undefined })
</script>
