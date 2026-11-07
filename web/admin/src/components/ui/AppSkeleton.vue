<template>
  <div role="status" class="rounded-2xl border border-slate-300 bg-white p-5  dark:border-slate-800 dark:bg-slate-900">
    <span class="sr-only">{{ label }}</span>
    <div class="animate-pulse space-y-4" aria-hidden="true">
      <div class="h-3 w-32 rounded bg-slate-200 dark:bg-slate-700" />
      <div class="grid grid-cols-1 gap-3 sm:grid-cols-3">
        <div v-for="n in 3" :key="n" class="h-16 rounded-xl bg-slate-100 dark:bg-slate-800" />
      </div>
      <div class="h-3 w-2/3 rounded bg-slate-100 dark:bg-slate-800" />
    </div>
  </div>
</template>

<script setup lang="ts">
defineProps<{ label: string }>()
</script>
