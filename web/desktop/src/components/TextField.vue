<script setup lang="ts">
defineProps<{
  id: string;
  label: string;
  icon: "user" | "tag";
  autocomplete: string;
}>();

const model = defineModel<string>({ required: true });
</script>

<template>
  <div class="field">
    <label :for="id">{{ label }}</label>
    <div class="control">
      <svg
        viewBox="0 0 24 24"
        fill="none"
        stroke="currentColor"
        stroke-width="1.8"
        stroke-linecap="round"
        stroke-linejoin="round"
        aria-hidden="true"
      >
        <template v-if="icon === 'user'">
          <circle cx="12" cy="8" r="3.5"/>
          <path d="M5 20c1.2-3.2 3.9-5 7-5s5.8 1.8 7 5"/>
        </template>
        <template v-else>
          <path d="M20.6 13.4L11 3.8A2 2 0 0 0 9.6 3.2H4a1 1 0 0 0-1 1v5.6c0 .5.2 1 .6 1.4l9.6 9.6a2 2 0 0 0 2.8 0l4.6-4.6a2 2 0 0 0 0-2.8z"/>
          <circle cx="8.5" cy="8.5" r="1.2"/>
        </template>
      </svg>
      <input :id="id" v-model="model" :autocomplete="autocomplete" />
    </div>
  </div>
</template>
