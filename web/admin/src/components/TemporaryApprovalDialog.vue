<template>
  <DialogRoot :open="open" @update:open="(value) => !value && cancel()">
    <DialogPortal>
      <DialogOverlay class="fixed inset-0 z-50 bg-black/60" />
      <DialogContent class="fixed left-1/2 top-1/2 z-50 w-[min(30rem,calc(100vw-2rem))] -translate-x-1/2 -translate-y-1/2 rounded-xl border border-slate-200 bg-white p-5 shadow-xl dark:border-slate-700 dark:bg-slate-900">
        <DialogTitle class="text-base font-semibold">{{ t('temporaryApproval.title') }}</DialogTitle>
        <DialogDescription class="mt-2 text-sm text-slate-600 dark:text-slate-400">{{ t('temporaryApproval.description', { namespace }) }}</DialogDescription>
        <p v-if="fingerprint" class="mt-3 break-all text-xs"><span class="font-semibold">{{ t('publishers.fingerprint') }} (SHA-256): </span><code>{{ fingerprint }}</code></p>
        <template v-if="challengeId">
          <p class="mt-3 text-sm text-slate-600 dark:text-slate-400">{{ t('temporaryApproval.instructions', { id: challengeId }) }}</p>
          <label class="mt-3 flex flex-col gap-1 text-sm">{{ t('temporaryApproval.code') }}
            <input v-model="code" inputmode="numeric" autocomplete="one-time-code" maxlength="8" class="rounded-md border border-slate-300 bg-white px-3 py-2 font-mono dark:border-slate-600 dark:bg-slate-800" />
          </label>
        </template>
        <p v-if="errorMessage" role="alert" class="mt-3 text-sm text-red-700 dark:text-red-300">{{ errorMessage }}</p>
        <div class="mt-5 flex flex-wrap justify-end gap-2">
          <AppButton :disabled="busy" @click="cancel">{{ t('states.cancel') }}</AppButton>
          <AppButton v-if="!challengeId" variant="primary" :loading="busy" @click="start">{{ t('temporaryApproval.generate') }}</AppButton>
          <AppButton v-else variant="primary" :loading="busy" :disabled="!/^\d{8}$/.test(code)" @click="confirm">{{ t('states.confirm') }}</AppButton>
        </div>
      </DialogContent>
    </DialogPortal>
  </DialogRoot>
</template>

<script setup lang="ts">
import { ref, watch } from 'vue'
import { useI18n } from 'vue-i18n'
import { DialogContent, DialogDescription, DialogOverlay, DialogPortal, DialogRoot, DialogTitle } from 'reka-ui'
import { ApiError, api } from '../api/client'
import AppButton from './ui/AppButton.vue'

const props = defineProps<{ open: boolean; publisherId: string; namespace: string; keyId: string | null }>()
const emit = defineEmits<{ cancelled: []; approved: [] }>()
const { t } = useI18n()
const challengeId = ref('')
const fingerprint = ref<string | null>(null)
const code = ref('')
const busy = ref(false)
const errorMessage = ref('')
watch(() => props.open, () => { challengeId.value = ''; fingerprint.value = null; code.value = ''; errorMessage.value = '' })
function cancel() { if (!busy.value) emit('cancelled') }
async function start() {
  busy.value = true
  errorMessage.value = ''
  try {
    const result = await api.startTemporaryApproval(props.publisherId, props.keyId)
    challengeId.value = result.challengeId
    fingerprint.value = result.fingerprint
  } catch (error) {
    errorMessage.value = error instanceof ApiError && error.status === 429 ? t('stepUp.rateLimited') : t('temporaryApproval.failed')
  } finally { busy.value = false }
}
async function confirm() {
  if (!/^\d{8}$/.test(code.value) || !challengeId.value) return
  busy.value = true
  errorMessage.value = ''
  try {
    await api.confirmTemporaryApproval(challengeId.value, code.value)
    code.value = ''
    emit('approved')
  } catch (error) {
    code.value = ''
    errorMessage.value = error instanceof ApiError && error.code === 'invalid_code' ? t('temporaryApproval.invalid') : t('temporaryApproval.failed')
  } finally { busy.value = false }
}
</script>
