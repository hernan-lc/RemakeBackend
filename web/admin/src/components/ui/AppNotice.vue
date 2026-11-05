<template>
  <div :role="tone === 'danger' ? 'alert' : 'note'" :class="['flex items-start gap-3 rounded-xl border px-4 py-3 text-sm leading-6', tones[tone]]">
    <AppIcon :name="iconName" :size="18" class="mt-0.5 shrink-0" aria-hidden="true" />
    <div class="min-w-0 flex-1">
      <p v-if="title" class="font-semibold">{{ title }}</p>
      <div :class="title ? 'mt-0.5' : undefined"><slot /></div>
    </div>
  </div>
</template>

<script setup lang="ts">
import { computed } from 'vue'
import AppIcon from '../icons/AppIcon.vue'
import type { IconName } from '../icons/icons'

type Tone = 'info' | 'warn' | 'danger' | 'success'
const props = withDefaults(defineProps<{ tone?: Tone; title?: string }>(), { tone: 'info', title: undefined })
const tones: Record<Tone, string> = {
  info: 'border-indigo-200 bg-indigo-50 text-indigo-900 dark:border-indigo-900/80 dark:bg-indigo-950/50 dark:text-indigo-200',
  warn: 'border-amber-200 bg-amber-50 text-amber-900 dark:border-amber-900/80 dark:bg-amber-950/40 dark:text-amber-200',
  danger: 'border-red-200 bg-red-50 text-red-900 dark:border-red-900/80 dark:bg-red-950/40 dark:text-red-200',
  success: 'border-emerald-200 bg-emerald-50 text-emerald-900 dark:border-emerald-900/80 dark:bg-emerald-950/40 dark:text-emerald-200',
}
const iconName = computed<IconName>(() => props.tone === 'success' ? 'checkCircle' : props.tone === 'info' ? 'info' : 'alert')
</script>
