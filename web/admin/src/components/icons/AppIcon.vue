<template>
  <svg
    :width="size"
    :height="size"
    viewBox="0 0 24 24"
    fill="none"
    stroke="currentColor"
    stroke-width="2"
    stroke-linecap="round"
    stroke-linejoin="round"
    focusable="false"
    :aria-hidden="label ? undefined : 'true'"
    :aria-label="label ?? undefined"
    :role="label ? 'img' : undefined"
    :class="spin ? 'animate-spin' : undefined"
    v-html="path"
  />
</template>

<script setup lang="ts">
import { computed } from 'vue'
import { iconPaths, type IconName } from './icons'

const props = withDefaults(
  defineProps<{ name: IconName; size?: number | string; label?: string; spin?: boolean }>(),
  { size: 18, label: undefined, spin: false },
)

const path = computed(() => iconPaths[props.name])
</script>
