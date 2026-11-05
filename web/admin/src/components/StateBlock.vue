<template>
  <AppSkeleton v-if="loading" :label="t('states.loading')" />
  <div
    v-else-if="error"
    role="alert"
    class="rounded-lg border border-red-200 bg-red-50 p-4 dark:border-red-900 dark:bg-red-950"
  >
    <p class="flex items-center gap-2 font-medium text-red-800 dark:text-red-200">
      <AppIcon name="alert" :size="17" aria-hidden="true" />
      {{ title ?? t('errors.loadFailed') }}
    </p>
    <p v-if="detail" class="mt-1 font-mono text-sm text-red-700 dark:text-red-300">{{ detail }}</p>
    <AppButton v-if="retry" size="sm" variant="secondary" icon="refresh" class="mt-3" @click="$emit('retry')">
      {{ t('states.retry') }}
    </AppButton>
  </div>
  <div v-else-if="empty">
    <EmptyState :title="emptyTitle ?? t('states.empty')" :hint="emptyHint" :icon="emptyIcon">
      <template v-if="$slots.emptyActions" #actions><slot name="emptyActions" /></template>
    </EmptyState>
  </div>
  <slot v-else />
</template>

<script setup lang="ts">
import { useI18n } from 'vue-i18n'
import AppIcon from './icons/AppIcon.vue'
import type { IconName } from './icons/icons'
import AppButton from './ui/AppButton.vue'
import EmptyState from './ui/EmptyState.vue'
import AppSkeleton from './ui/AppSkeleton.vue'

withDefaults(
  defineProps<{
    loading: boolean
    error: boolean
    empty?: boolean
    title?: string
    detail?: string
    retry?: boolean
    emptyTitle?: string
    emptyHint?: string
    emptyIcon?: IconName
  }>(),
  { empty: false, title: undefined, detail: undefined, retry: false, emptyTitle: undefined, emptyHint: undefined, emptyIcon: 'inbox' },
)

defineEmits<{ retry: [] }>()

const { t } = useI18n()
</script>
