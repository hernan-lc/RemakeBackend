<script setup lang="ts">
import { onMounted, onUnmounted, ref } from "vue";
import { LANGUAGES, NATIVE_NAMES, type Language } from "../i18n";

const props = defineProps<{ lang: Language }>();
const emit = defineEmits<{ select: [lang: Language] }>();

const open = ref(false);

function toggle(event: Event): void {
  event.stopPropagation();
  open.value = !open.value;
}

function choose(code: Language): void {
  emit("select", code);
  open.value = false;
}

function onDocumentClick(event: MouseEvent): void {
  if (!(event.target instanceof Node && root.value?.contains(event.target))) {
    open.value = false;
  }
}

function onEscape(event: KeyboardEvent): void {
  if (event.key === "Escape") open.value = false;
}

const root = ref<HTMLElement | null>(null);

onMounted(() => {
  document.addEventListener("click", onDocumentClick);
  document.addEventListener("keydown", onEscape);
});

onUnmounted(() => {
  document.removeEventListener("click", onDocumentClick);
  document.removeEventListener("keydown", onEscape);
});
</script>

<template>
  <div class="langselect" ref="root">
    <button
      id="lang-button"
      type="button"
      aria-haspopup="listbox"
      :aria-expanded="open ? 'true' : 'false'"
      @click="toggle"
    >
      <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
        <circle cx="12" cy="12" r="8.5"/>
        <path d="M3.5 12h17M12 3.5c2.5 2.3 3.8 5.2 3.8 8.5s-1.3 6.2-3.8 8.5c-2.5-2.3-3.8-5.2-3.8-8.5s1.3-6.2 3.8-8.5z"/>
      </svg>
      <span id="lang-current">{{ NATIVE_NAMES[props.lang] }}</span>
      <svg class="chev" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
        <path d="M6 9l6 6 6-6"/>
      </svg>
    </button>
    <ul id="lang-menu" role="listbox" :hidden="!open">
      <li
        v-for="code in LANGUAGES"
        :key="code"
        role="option"
        :aria-selected="code === props.lang ? 'true' : 'false'"
        :class="{ active: code === props.lang }"
        @click="choose(code)"
      >
        {{ NATIVE_NAMES[code] }}
      </li>
    </ul>
  </div>
</template>
