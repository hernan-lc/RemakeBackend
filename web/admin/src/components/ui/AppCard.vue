<template>
  <section :aria-label="label" :class="classes">
    <div v-if="title || $slots.actions" class="mb-3 flex flex-wrap items-center gap-2">
      <h2 v-if="title" class="text-base font-semibold tracking-tight text-slate-900 dark:text-slate-100">{{ title }}</h2>
      <div v-if="$slots.actions" class="ml-auto flex flex-wrap items-center gap-2">
        <slot name="actions" />
      </div>
    </div>
    <slot />
  </section>
</template>

<script setup lang="ts">
import { computed } from 'vue'

const props = withDefaults(defineProps<{ title?: string; label?: string; padded?: boolean }>(), {
  title: undefined,
  label: undefined,
  padded: true,
})

const classes = computed(() => [
  'rounded-2xl border border-slate-200/80 bg-white/95 text-slate-900 shadow-[0_10px_30px_-24px_rgba(15,23,42,0.5)]',
  'dark:border-slate-800 dark:bg-slate-900/90 dark:text-slate-100',
  props.padded ? 'p-5 sm:p-6' : undefined,
])
</script>
