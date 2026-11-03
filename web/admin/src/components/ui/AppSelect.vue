<template>
  <label class="flex min-w-0 flex-col gap-1 text-sm">
    <span v-if="label" class="font-medium text-slate-700 dark:text-slate-300">{{ label }}</span>
    <span class="relative inline-flex items-center">
      <select
        :value="modelValue"
        :class="selectClasses"
        @change="$emit('update:modelValue', ($event.target as HTMLSelectElement).value)"
      >
        <slot />
      </select>
      <AppIcon name="chevronDown" :size="15" aria-hidden="true" class="pointer-events-none absolute right-2.5 text-slate-400 dark:text-slate-500" />
    </span>
  </label>
</template>

<script setup lang="ts">
import { computed } from 'vue'
import AppIcon from '../icons/AppIcon.vue'

defineProps<{ modelValue: string; label?: string }>()
defineEmits<{ 'update:modelValue': [value: string] }>()

const selectClasses = computed(() => [
  'w-full appearance-none rounded-md border border-slate-300 bg-white py-2 pl-3 pr-9 text-sm text-slate-900',
  'hover:border-slate-400 focus:border-slate-500 focus:outline-none',
  'dark:border-slate-600 dark:bg-slate-800 dark:text-slate-100 dark:hover:border-slate-500 dark:focus:border-slate-400',
  '[&>option]:bg-white dark:[&>option]:bg-slate-800',
])
</script>
