<template>
  <label class="relative block w-full min-w-0 sm:max-w-sm">
    <span class="sr-only">{{ label }}</span>
    <AppIcon name="search" :size="17" class="pointer-events-none absolute left-3 top-1/2 -translate-y-1/2 text-slate-400" aria-hidden="true" />
    <input :value="modelValue" type="search" :aria-label="label" :placeholder="placeholder ?? label"
      class="min-h-10 w-full rounded-lg border border-slate-200 bg-white py-2 pl-10 pr-3 text-sm text-slate-900 outline-none transition-colors placeholder:text-slate-400 hover:border-slate-300 focus:border-indigo-500 focus:ring-2 focus:ring-indigo-500/15 dark:border-slate-700 dark:bg-slate-900 dark:text-white dark:hover:border-slate-600 dark:focus:border-indigo-400"
      @input="$emit('update:modelValue', ($event.target as HTMLInputElement).value)" />
  </label>
</template>

<script setup lang="ts">
import AppIcon from '../icons/AppIcon.vue'
defineProps<{ modelValue: string; label: string; placeholder?: string }>()
defineEmits<{ 'update:modelValue': [value: string] }>()
</script>
