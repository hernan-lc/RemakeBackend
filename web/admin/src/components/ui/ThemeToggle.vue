<template>
  <button
    type="button"
    :aria-pressed="isDark"
    :aria-label="t('app.toggleTheme')"
    :title="t(isDark ? 'app.themeLight' : 'app.themeDark')"
    class="inline-flex size-11 items-center justify-center rounded-xl text-slate-600 transition-colors hover:bg-slate-100 hover:text-slate-900 dark:text-slate-300 dark:hover:bg-slate-800 dark:hover:text-white"
    @click="toggle"
  >
    <AppIcon :name="isDark ? 'sun' : 'moon'" :size="18" aria-hidden="true" />
  </button>
</template>

<script setup lang="ts">
import { computed } from 'vue'
import { useI18n } from 'vue-i18n'
import { useUiStore } from '../../stores/ui'
import AppIcon from '../icons/AppIcon.vue'

const { t } = useI18n()
const ui = useUiStore()
const isDark = computed(() => ui.theme === 'dark')

function toggle() {
  ui.toggleTheme()
}
</script>
