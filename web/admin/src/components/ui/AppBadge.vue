<template>
  <span :class="classes" :title="title">
    <AppIcon v-if="icon" :name="icon" :size="13" aria-hidden="true" />
    <slot />
  </span>
</template>

<script setup lang="ts">
import { computed } from 'vue'
import AppIcon from '../icons/AppIcon.vue'
import type { IconName } from '../icons/icons'

type Tone = 'neutral' | 'success' | 'warn' | 'danger' | 'info'

const props = withDefaults(defineProps<{ tone?: Tone; icon?: IconName; title?: string }>(), {
  tone: 'neutral',
  icon: undefined,
  title: undefined,
})

const tones: Record<Tone, string> = {
  neutral: 'bg-slate-100 text-slate-700 dark:bg-slate-800 dark:text-slate-300',
  success: 'bg-emerald-100 text-emerald-800 dark:bg-emerald-950 dark:text-emerald-300',
  warn: 'bg-amber-100 text-amber-800 dark:bg-amber-950 dark:text-amber-300',
  danger: 'bg-red-100 text-red-800 dark:bg-red-950 dark:text-red-300',
  info: 'bg-sky-100 text-sky-800 dark:bg-sky-950 dark:text-sky-300',
}

const classes = computed(() => [
  'inline-flex max-w-full items-center gap-1 rounded-full px-2.5 py-1 text-[11px] font-semibold tracking-wide',
  tones[props.tone],
])
</script>
