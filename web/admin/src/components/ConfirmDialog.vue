<template>
  <DialogRoot :open="open" @update:open="(v) => !v && $emit('cancel')">
    <DialogPortal>
      <DialogOverlay class="fixed inset-0 bg-black/40" />
      <DialogContent
        class="fixed left-1/2 top-1/2 w-[min(28rem,90vw)] -translate-x-1/2 -translate-y-1/2 rounded-lg bg-white p-6 shadow-lg"
        @escape-key-down="$emit('cancel')"
      >
        <DialogTitle class="text-base font-semibold text-slate-900">{{ title }}</DialogTitle>
        <DialogDescription class="mt-2 text-sm text-slate-600">{{ body }}</DialogDescription>
        <div class="mt-5 flex justify-end gap-2">
          <button
            type="button"
            class="rounded border border-slate-300 px-3 py-1.5 text-sm font-medium text-slate-700 hover:bg-slate-50"
            @click="$emit('cancel')"
          >
            {{ t('states.cancel') }}
          </button>
          <button
            type="button"
            :disabled="busy"
            class="rounded bg-red-700 px-3 py-1.5 text-sm font-medium text-white hover:bg-red-600 disabled:opacity-50"
            @click="$emit('confirm')"
          >
            {{ t('states.confirm') }}
          </button>
        </div>
      </DialogContent>
    </DialogPortal>
  </DialogRoot>
</template>

<script setup lang="ts">
import { useI18n } from 'vue-i18n'
import { DialogContent, DialogDescription, DialogOverlay, DialogPortal, DialogRoot, DialogTitle } from 'reka-ui'

defineProps<{ open: boolean; title: string; body: string; busy?: boolean }>()
defineEmits<{ confirm: []; cancel: [] }>()

const { t } = useI18n()
</script>
