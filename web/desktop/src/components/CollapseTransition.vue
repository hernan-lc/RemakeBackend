<script setup lang="ts">
// Height-animated show/hide without measuring: the grid-rows 0fr/1fr trick
// lets the card grow and shrink smoothly instead of jumping when fields
// mount or unmount. Content is inert and hidden from assistive tech while
// collapsed; motion is instant under prefers-reduced-motion (see style.css).
defineProps<{ open: boolean }>();
</script>

<template>
  <div class="collapse" :class="{ open }" :inert="!open" :aria-hidden="!open ? 'true' : undefined">
    <div class="collapse-inner">
      <slot />
    </div>
  </div>
</template>
