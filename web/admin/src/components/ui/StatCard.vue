<template>
  <component :is="to ? RouterLink : 'div'" :to="to"
    :class="['group flex min-h-28 flex-col justify-between rounded-xl border border-slate-200/80 bg-white p-4 dark:border-slate-800 dark:bg-slate-900', to ? 'transition-colors hover:border-indigo-300 hover:bg-indigo-50/30 dark:hover:border-indigo-800 dark:hover:bg-indigo-500/5' : '']">
    <div class="flex items-center justify-between gap-2">
      <dt class="text-xs font-medium text-slate-500 dark:text-slate-400">{{ label }}</dt>
      <AppIcon :name="icon" :size="16" :class="toneClasses[tone]" aria-hidden="true" />
    </div>
    <div class="mt-3 flex items-end justify-between gap-2">
      <dd class="text-[28px] font-semibold leading-none tracking-tight tabular-nums text-slate-950 dark:text-white">{{ displayValue }}</dd>
      <AppIcon v-if="to" name="arrowRight" :size="15" class="text-slate-400 opacity-0 transition-opacity group-hover:opacity-100 group-focus-visible:opacity-100" aria-hidden="true" />
    </div>
    <p v-if="hint" class="mt-2 text-xs leading-4 text-slate-500 dark:text-slate-400">{{ hint }}</p>
  </component>
</template>

<script setup lang="ts">
import { computed } from 'vue'
import { RouterLink } from 'vue-router'
import AppIcon from '../icons/AppIcon.vue'
import type { IconName } from '../icons/icons'

type Tone = 'indigo' | 'amber' | 'emerald' | 'sky'
const props = withDefaults(
  defineProps<{ label: string; value: number | string; icon: IconName; hint?: string; to?: string; tone?: Tone }>(),
  { hint: undefined, to: undefined, tone: 'indigo' },
)
const displayValue = computed(() => (typeof props.value === 'number' ? props.value.toLocaleString() : props.value))
const toneClasses: Record<Tone, string> = {
  indigo: 'text-indigo-600 dark:text-indigo-400',
  amber: 'text-amber-600 dark:text-amber-400',
  emerald: 'text-emerald-600 dark:text-emerald-400',
  sky: 'text-sky-600 dark:text-sky-400',
}
</script>
