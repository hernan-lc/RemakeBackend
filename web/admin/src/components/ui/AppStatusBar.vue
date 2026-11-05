<template>
  <div class="space-y-2">
    <div class="flex items-center justify-between gap-3 text-sm">
      <span class="min-w-0 truncate text-slate-600 dark:text-slate-300">{{ label }}</span>
      <span class="font-semibold tabular-nums text-slate-900 dark:text-white">{{ value.toLocaleString() }}</span>
    </div>
    <div class="h-1.5 overflow-hidden rounded-full bg-slate-100 dark:bg-slate-800" role="meter" :aria-label="label" :aria-valuemin="0" :aria-valuemax="Math.max(total, 1)" :aria-valuenow="value">
      <div :class="['h-full rounded-full transition-[width] duration-300', fills[tone]]" :style="{ width: percentage + '%' }" />
    </div>
  </div>
</template>

<script setup lang="ts">
import { computed } from 'vue'
type Tone = 'info' | 'success' | 'warn' | 'danger'
const props = withDefaults(defineProps<{ label: string; value: number; total: number; tone?: Tone }>(), { tone: 'info' })
const percentage = computed(() => props.total > 0 ? Math.min(100, Math.max(0, (props.value / props.total) * 100)) : 0)
const fills: Record<Tone, string> = { info: 'bg-indigo-500', success: 'bg-emerald-500', warn: 'bg-amber-500', danger: 'bg-rose-500' }
</script>
