<template>
  <component :is="to ? RouterLink : 'div'" :to="to" :class="classes">
    <div class="flex items-center gap-2">
      <span class="rounded-md bg-slate-100 p-1.5 text-slate-600 dark:bg-slate-800 dark:text-slate-300">
        <AppIcon :name="icon" :size="18" aria-hidden="true" />
      </span>
      <dt class="min-w-0 flex-1 text-sm text-slate-600 dark:text-slate-400">{{ label }}</dt>
    </div>
    <dd class="mt-2 text-3xl font-bold tabular-nums text-slate-900 dark:text-white">{{ displayValue }}</dd>
    <p v-if="hint" class="mt-1 text-xs text-slate-500 dark:text-slate-500">{{ hint }}</p>
  </component>
</template>

<script setup lang="ts">
import { computed } from 'vue'
import { RouterLink } from 'vue-router'
import AppIcon from '../icons/AppIcon.vue'
import type { IconName } from '../icons/icons'

const props = withDefaults(
  defineProps<{ label: string; value: number | string; icon: IconName; hint?: string; to?: string }>(),
  { hint: undefined, to: undefined },
)

const displayValue = computed(() => (typeof props.value === 'number' ? props.value.toLocaleString() : props.value))

const classes = computed(() => [
  'block rounded-lg border border-slate-200 bg-white p-4',
  'dark:border-slate-700 dark:bg-slate-900',
  props.to ? 'transition-colors hover:border-slate-300 hover:bg-slate-50 dark:hover:border-slate-600 dark:hover:bg-slate-800' : undefined,
])
</script>
