<script setup lang="ts">
import { ref } from "vue";

defineProps<{
  id: string;
  label: string;
  autocomplete: string;
  showLabel: string;
  hideLabel: string;
}>();

const model = defineModel<string>({ required: true });
const visible = ref(false);

function toggle(): void {
  visible.value = !visible.value;
}
</script>

<template>
  <div class="field password-field">
    <label :for="id">{{ label }}</label>
    <div class="control">
      <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
        <rect x="4.5" y="10.5" width="15" height="9.5" rx="2"/>
        <path d="M8 10.5V7a4 4 0 0 1 8 0v3.5"/>
      </svg>
      <input :id="id" v-model="model" :type="visible ? 'text' : 'password'" :autocomplete="autocomplete" />
      <button
        type="button"
        class="peek"
        :aria-pressed="visible ? 'true' : 'false'"
        :aria-label="visible ? hideLabel : showLabel"
        @click="toggle"
      >
        <svg v-if="!visible" class="eye" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
          <path d="M2.5 12S6 5.5 12 5.5 21.5 12 21.5 12 18 18.5 12 18.5 2.5 12 2.5 12z"/>
          <circle cx="12" cy="12" r="3"/>
        </svg>
        <svg v-else class="eye-off" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
          <path d="M4 4l16 16"/>
          <path d="M10.6 5.9A9.8 9.8 0 0 1 12 5.5c6 0 9.5 6.5 9.5 6.5a17.6 17.6 0 0 1-3.3 3.9M6.6 6.6C4 8.3 2.5 12 2.5 12S6 18.5 12 18.5c1.5 0 2.9-.4 4.1-1"/>
          <path d="M9.9 9.9a3 3 0 0 0 4.2 4.2"/>
        </svg>
      </button>
    </div>
  </div>
</template>
