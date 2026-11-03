<template>
  <DialogRoot :open="open" @update:open="(v) => !v && onCancel()">
    <DialogPortal>
      <DialogOverlay class="fixed inset-0 z-50 bg-black/50 dark:bg-black/70" />
      <DialogContent
        class="fixed left-1/2 top-1/2 z-50 w-[min(28rem,calc(100vw-2rem))] -translate-x-1/2 -translate-y-1/2 rounded-xl border border-slate-200 bg-white p-5 shadow-xl dark:border-slate-700 dark:bg-slate-900"
        @escape-key-down="onCancel"
      >
        <DialogTitle class="flex items-center gap-2 text-base font-semibold text-slate-900 dark:text-white">
          <AppIcon name="lock" :size="18" aria-hidden="true" class="text-slate-500 dark:text-slate-400" />
          {{ t('stepUp.title') }}
        </DialogTitle>
        <DialogDescription class="mt-2 text-sm text-slate-600 dark:text-slate-400">
          {{ t('stepUp.body', { action: actionLabel }) }}
        </DialogDescription>
        <div class="mt-4 flex flex-col gap-3">
          <AppButton variant="secondary" icon="external" :loading="busy === 'google'" :disabled="busy !== null" class="w-full" @click="startGoogle">
            {{ t('stepUp.continueWithGoogle') }}
          </AppButton>
          <div class="flex items-center gap-3 text-xs text-slate-500 dark:text-slate-400" aria-hidden="true">
            <span class="h-px flex-1 bg-slate-200 dark:bg-slate-700" />
            {{ t('stepUp.orPassword') }}
            <span class="h-px flex-1 bg-slate-200 dark:bg-slate-700" />
          </div>
          <form class="flex flex-col gap-3" @submit.prevent="submitPassword">
            <AppInput
              v-model="password"
              :label="t('stepUp.password')"
              type="password"
              required
              autocomplete="current-password"
              icon="lock"
              :disabled="busy !== null"
            />
            <p v-if="error" role="alert" class="flex items-start gap-2 text-sm text-red-700 dark:text-red-300">
              <AppIcon name="alert" :size="16" class="mt-0.5 shrink-0" aria-hidden="true" />
              {{ error }}
            </p>
            <div class="flex flex-wrap justify-end gap-2">
              <AppButton variant="secondary" :disabled="busy !== null" @click="onCancel">
                {{ t('states.cancel') }}
              </AppButton>
              <AppButton type="submit" variant="primary" :loading="busy === 'password'" :disabled="busy !== null || !password">
                {{ t('stepUp.verify') }}
              </AppButton>
            </div>
          </form>
          <p class="text-xs text-slate-500 dark:text-slate-400">{{ t('stepUp.ephemeral') }}</p>
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
import type { StepUpAction } from '../api/types'
import AppIcon from './icons/AppIcon.vue'
import AppButton from './ui/AppButton.vue'
import AppInput from './ui/AppInput.vue'

const props = defineProps<{
  open: boolean
  action: StepUpAction
  actionLabel: string
  publisherId: string
  keyId?: string | null
  returnTo: string
}>()

const emit = defineEmits<{
  (e: 'granted', grantId: string): void
  (e: 'cancelled'): void
}>()

const { t } = useI18n()
const password = ref('')
const busy = ref<'password' | 'google' | null>(null)
const error = ref('')

// Credentials are ephemeral: clearing on every open, close, success, and
// cancellation. Nothing is ever written to web storage.
function reset() {
  password.value = ''
  busy.value = null
  error.value = ''
}

watch(
  () => props.open,
  (open) => {
    if (open) reset()
  },
)

function onCancel() {
  if (busy.value) return
  reset()
  emit('cancelled')
}

function failure(e: unknown): string {
  if (e instanceof ApiError) {
    if (e.status === 401) return t('stepUp.invalidPassword')
    if (e.status === 403 && e.code === 'password_unavailable') return t('stepUp.noPassword')
    if (e.status === 403) return t('stepUp.forbidden')
    if (e.status === 429) return t('stepUp.rateLimited')
  }
  return t('errors.actionFailed')
}

async function submitPassword() {
  if (!password.value || busy.value) return
  busy.value = 'password'
  error.value = ''
  try {
    const grant = await api.stepUpPassword(props.action, props.publisherId, props.keyId ?? null, password.value)
    reset()
    emit('granted', grant.grantId)
  } catch (e) {
    password.value = ''
    error.value = failure(e)
    busy.value = null
  }
}

async function startGoogle() {
  if (busy.value) return
  busy.value = 'google'
  error.value = ''
  try {
    const start = await api.stepUpStart(props.action, props.publisherId, props.keyId ?? null, props.returnTo)
    // Full navigation: the provider round-trip returns to `returnTo` with the
    // one-time grant in the query. The password never leaves this form.
    password.value = ''
    window.location.href = start.authorizationUrl
  } catch (e) {
    error.value = failure(e)
    busy.value = null
  }
}
</script>
