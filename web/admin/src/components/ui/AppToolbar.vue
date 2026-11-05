<template>
  <div class="flex flex-col gap-2 sm:flex-row sm:items-center">
    <AppSearch :model-value="modelValue" :label="searchLabel" class="min-w-0 flex-1 sm:max-w-sm" @update:model-value="$emit('update:modelValue', $event)" />
    <div v-if="$slots.filters" class="flex flex-wrap items-center gap-2"><slot name="filters" /></div>
    <AppButton size="sm" variant="ghost" icon="refresh" icon-only :label="refreshLabel" :title="refreshLabel" @click="$emit('refresh')" />
  </div>
</template>

<script setup lang="ts">
import AppSearch from './AppSearch.vue'
import AppButton from './AppButton.vue'

defineProps<{ modelValue: string; searchLabel: string; refreshLabel: string }>()
defineEmits<{ 'update:modelValue': [value: string]; refresh: [] }>()
</script>
