<template>
  <div v-if="loading" role="status" class="rounded border border-slate-200 bg-white p-4 text-slate-700">
    {{ t('states.loading') }}
  </div>
  <div v-else-if="error" role="alert" class="rounded border border-red-200 bg-red-50 p-4">
    <p class="font-medium text-red-800">{{ title ?? t('errors.loadFailed') }}</p>
    <p v-if="detail" class="mt-1 text-sm text-red-700">{{ detail }}</p>
    <button
      v-if="retry"
      type="button"
      class="mt-3 rounded bg-slate-900 px-3 py-1.5 text-sm font-medium text-white hover:bg-slate-700"
      @click="$emit('retry')"
    >
      {{ t('states.retry') }}
    </button>
  </div>
  <div v-else-if="empty" class="rounded border border-dashed border-slate-300 p-6 text-center text-slate-500">
    {{ t('states.empty') }}
  </div>
  <slot v-else />
</template>

<script setup lang="ts">
import { useI18n } from 'vue-i18n'

defineProps<{ loading: boolean; error: boolean; empty?: boolean; title?: string; detail?: string; retry?: boolean }>()
defineEmits<{ retry: [] }>()

const { t } = useI18n()
</script>
