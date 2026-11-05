<template>
  <component :is="to ? RouterLink : 'div'" :to="to"
    :class="['group relative block overflow-hidden rounded-2xl border border-slate-200/80 bg-white/95 p-5 shadow-[0_10px_30px_-24px_rgba(15,23,42,0.5)] dark:border-slate-800 dark:bg-slate-900/90', to ? 'transition-all hover:-translate-y-0.5 hover:border-indigo-200 hover:shadow-md dark:hover:border-indigo-800' : '']">
    <div class="flex items-start justify-between gap-3">
      <span :class="['inline-flex size-11 items-center justify-center rounded-xl', toneClasses[tone]]">
        <AppIcon :name="icon" :size="20" aria-hidden="true" />
      </span>
      <AppIcon v-if="to" name="arrowRight" :size="16" class="text-slate-300 transition-transform group-hover:translate-x-1 dark:text-slate-600" aria-hidden="true" />
    </div>
    <dl class="mt-4">
      <dd class="text-[32px] font-bold leading-none tracking-tight tabular-nums text-slate-950 dark:text-white">{{ displayValue }}</dd>
      <dt class="mt-2 text-sm font-semibold text-slate-700 dark:text-slate-200">{{ label }}</dt>
    </dl>
    <p v-if="hint" class="mt-1 text-xs leading-5 text-slate-500 dark:text-slate-400">{{ hint }}</p>
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
  indigo: 'bg-indigo-50 text-indigo-600 dark:bg-indigo-500/15 dark:text-indigo-300',
  amber: 'bg-amber-50 text-amber-600 dark:bg-amber-500/15 dark:text-amber-300',
  emerald: 'bg-emerald-50 text-emerald-600 dark:bg-emerald-500/15 dark:text-emerald-300',
  sky: 'bg-sky-50 text-sky-600 dark:bg-sky-500/15 dark:text-sky-300',
}
</script>
