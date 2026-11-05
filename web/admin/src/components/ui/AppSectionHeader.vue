<template>
  <div class="flex flex-wrap items-start justify-between gap-3">
    <div class="min-w-0">
      <h2 class="text-base font-semibold tracking-tight text-slate-900 sm:text-lg dark:text-slate-100">{{ title }}</h2>
      <p v-if="description" class="mt-1 max-w-2xl text-sm leading-6 text-slate-500 dark:text-slate-400">{{ description }}</p>
    </div>
    <div v-if="$slots.actions" class="flex shrink-0 items-center gap-2"><slot name="actions" /></div>
  </div>
</template>

<script setup lang="ts">
defineProps<{ title: string; description?: string }>()
</script>
