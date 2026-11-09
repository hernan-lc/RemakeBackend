<template>
  <DialogRoot :open="open" @update:open="(v) => !v && $emit('cancel')">
    <DialogPortal>
      <DialogOverlay class="fixed inset-0 z-50 bg-black/50 dark:bg-black/70" />
      <DialogContent
        class="fixed left-1/2 top-1/2 z-50 max-h-[calc(100dvh-2rem)] w-[min(32rem,calc(100vw-2rem))] overflow-y-auto -translate-x-1/2 -translate-y-1/2 rounded-xl border border-slate-300 bg-white p-5 sm:p-6 dark:border-slate-700 dark:bg-slate-900"
        @escape-key-down="$emit('cancel')"
      >
        <DialogTitle class="flex items-center gap-2 text-base font-semibold text-slate-900 dark:text-white">
          <AppIcon :name="danger ? 'alert' : 'info'" :size="18" aria-hidden="true" :class="danger ? 'text-red-600 dark:text-red-400' : 'text-slate-500 dark:text-slate-400'" />
          {{ title }}
        </DialogTitle>
        <DialogDescription class="mt-2 text-sm text-slate-600 dark:text-slate-400">{{ body }}</DialogDescription>
        <p v-if="error" role="alert" class="mt-4 rounded-lg bg-red-50 p-3 text-sm text-red-800 dark:bg-red-950 dark:text-red-200">{{ error }}</p>
        <label v-if="promptLabel" class="mt-4 flex flex-col gap-1 text-sm">
          <span class="font-medium text-slate-700 dark:text-slate-300">{{ promptLabel }}</span>
          <input
            :value="promptValue"
            :placeholder="promptPlaceholder"
            maxlength="500"
            class="w-full rounded-md border border-slate-300 bg-white px-3 py-2 text-sm text-slate-900 placeholder:text-slate-400 focus:border-slate-500 focus:outline-none dark:border-slate-600 dark:bg-slate-800 dark:text-slate-100 dark:placeholder:text-slate-500 dark:focus:border-slate-400"
            @input="onPromptInput"
          />
        </label>
        <div class="mt-5 flex flex-wrap justify-end gap-2">
          <AppButton variant="secondary" @click="$emit('cancel')">
            {{ t('states.cancel') }}
          </AppButton>
          <AppButton :variant="danger ? 'dangerFill' : 'primary'" :loading="busy" :disabled="busy || confirmDisabled || (promptLabel != null && !promptValue.trim()) || (confirmMatch != null && promptValue !== confirmMatch)" @click="$emit('confirm')">
            {{ t('states.confirm') }}
          </AppButton>
        </div>
      </DialogContent>
    </DialogPortal>
  </DialogRoot>
</template>

<script setup lang="ts">
import { useI18n } from 'vue-i18n'
import { DialogContent, DialogDescription, DialogOverlay, DialogPortal, DialogRoot, DialogTitle } from 'reka-ui'
import AppIcon from './icons/AppIcon.vue'
import AppButton from './ui/AppButton.vue'

withDefaults(
  defineProps<{
    open: boolean
    title: string
    body: string
    busy?: boolean
    confirmDisabled?: boolean
    danger?: boolean
    promptLabel?: string
    promptValue?: string
    promptPlaceholder?: string
    confirmMatch?: string
    error?: string
  }>(),
  { busy: false, confirmDisabled: false, danger: true, promptLabel: undefined, promptValue: '', promptPlaceholder: undefined, confirmMatch: undefined, error: undefined },
)

const emit = defineEmits<{
  (e: 'confirm'): void
  (e: 'cancel'): void
  (e: 'update:promptValue', value: string): void
}>()

const { t } = useI18n()

function onPromptInput(event: Event) {
  emit('update:promptValue', (event.target as HTMLInputElement).value)
}</script>
