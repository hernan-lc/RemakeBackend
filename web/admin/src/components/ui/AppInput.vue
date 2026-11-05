<template>
  <label class="flex min-w-0 flex-col gap-1 text-sm">
    <span v-if="label" class="font-medium text-slate-700 dark:text-slate-300">{{ label }}</span>
    <span class="relative inline-flex items-center">
      <AppIcon v-if="icon" :name="icon" :size="16" aria-hidden="true" class="pointer-events-none absolute left-3 text-slate-400 dark:text-slate-500" />
      <input
        :value="modelValue"
        :type="type"
        :required="required"
        :placeholder="placeholder"
        :autocomplete="autocomplete"
        :maxlength="maxlength"
        :class="inputClasses"
        @input="$emit('update:modelValue', ($event.target as HTMLInputElement).value)"
      />
    </span>
  </label>
</template>

<script setup lang="ts">
import { computed } from 'vue'
import AppIcon from '../icons/AppIcon.vue'
import type { IconName } from '../icons/icons'

const props = withDefaults(
  defineProps<{
    modelValue: string
    label?: string
    type?: string
    required?: boolean
    placeholder?: string
    autocomplete?: string
    maxlength?: number
    icon?: IconName
  }>(),
  { label: undefined, type: 'text', required: false, placeholder: undefined, autocomplete: undefined, maxlength: undefined, icon: undefined },
)

defineEmits<{ 'update:modelValue': [value: string] }>()

const inputClasses = computed(() => [
  'min-h-11 w-full rounded-xl border border-slate-200 bg-white px-3 py-2.5 text-sm text-slate-900 placeholder:text-slate-400 transition-colors',
  'hover:border-slate-300 focus:border-indigo-500 focus:outline-none focus:ring-2 focus:ring-indigo-500/15',
  'dark:border-slate-700 dark:bg-slate-800 dark:text-slate-100 dark:placeholder:text-slate-500 dark:hover:border-slate-600 dark:focus:border-indigo-400',
  props.icon ? 'pl-9' : undefined,
])
</script>
