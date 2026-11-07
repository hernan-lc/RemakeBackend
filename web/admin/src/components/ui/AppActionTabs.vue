<template>
  <div class="flex flex-wrap gap-1 rounded-lg bg-slate-100 p-1 dark:bg-slate-800" role="group" :aria-label="label">
    <button v-for="item in items" :key="item.value" type="button" :aria-pressed="modelValue === item.value"
      :class="['inline-flex min-h-11 items-center gap-1.5 rounded-md px-3 text-sm font-medium transition-colors',
        modelValue === item.value
          ? 'border border-slate-300 bg-white text-slate-950  dark:bg-slate-700 dark:text-white'
          : 'text-slate-600 hover:bg-white dark:text-slate-300 dark:hover:bg-slate-700']"
      @click="$emit('update:modelValue', modelValue === item.value ? '' : item.value)">
      <AppIcon v-if="item.icon" :name="item.icon" :size="15" aria-hidden="true" />
      {{ item.label }}
    </button>
  </div>
</template>

<script setup lang="ts">
import AppIcon from '../icons/AppIcon.vue'
import type { IconName } from '../icons/icons'

defineProps<{ label: string; modelValue: string; items: { value: string; label: string; icon?: IconName }[] }>()
defineEmits<{ 'update:modelValue': [value: string] }>()
</script>
