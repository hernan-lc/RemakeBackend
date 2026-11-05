<template>
  <div class="flex flex-wrap gap-1 rounded-lg bg-slate-100 p-1 dark:bg-slate-800/70" role="group" :aria-label="label">
    <button v-for="item in items" :key="item.value" type="button" :aria-pressed="modelValue === item.value"
      :class="['inline-flex min-h-9 items-center gap-1.5 rounded-md px-3 text-xs font-medium transition-colors',
        modelValue === item.value
          ? 'bg-white text-slate-950 shadow-sm dark:bg-slate-700 dark:text-white'
          : 'text-slate-600 hover:bg-white/70 dark:text-slate-300 dark:hover:bg-slate-700/70']"
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
