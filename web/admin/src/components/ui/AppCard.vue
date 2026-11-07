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
  'rounded-xl border border-slate-300 bg-white text-slate-900',
  'dark:border-slate-800 dark:bg-slate-900 dark:text-slate-100',
  props.padded ? 'p-4 sm:p-5' : undefined,
])
</script>
