<template>
  <div class="flex flex-col gap-6">
    <PageHeader :title="t('portal.title')" :description="t('portal.description')" icon="portal" />

    <AppCard :label="t('portal.requestTitle')" class="border-l-4 !border-l-indigo-500">
      <div class="grid gap-5 lg:grid-cols-[minmax(0,1fr)_minmax(18rem,1fr)] lg:items-center">
      <div><h2 class="text-base font-semibold">{{ t('portal.requestTitle') }}</h2><p class="mt-1 text-sm text-slate-500 dark:text-slate-400">{{ t('portal.namespaceHint') }}</p></div>
      <form class="flex flex-col gap-2" @submit.prevent="requestNamespace">
        <AppInput v-model="namespace" :label="t('portal.namespace')" required :maxlength="63" />
        <p v-if="requestError" role="alert" class="flex items-start gap-2 text-sm text-red-700 dark:text-red-300">
          <AppIcon name="alert" :size="16" class="mt-0.5 shrink-0" aria-hidden="true" />
          {{ requestError }}
        </p>
        <div>
          <AppButton type="submit" size="sm" variant="primary" :loading="requesting" :disabled="requesting || !validNamespace">
            {{ t('portal.requestButton') }}
          </AppButton>
        </div>
      </form>
      </div>
    </AppCard>

    <StateBlock :loading="isPending" :error="!!error" :retry="true" @retry="refetch">
      <div class="mb-4 flex flex-wrap items-end justify-between gap-2"><div><h2 class="text-lg font-semibold tracking-tight">{{ t('portal.yourNamespaces') }}</h2><p class="mt-1 text-sm text-slate-500 dark:text-slate-400">{{ t('portal.namespaceDescription') }}</p></div><span class="rounded-full bg-slate-200 px-2.5 py-1 text-xs font-semibold text-slate-600 dark:bg-slate-800 dark:text-slate-300">{{ publishers.length }}</span></div>
      <ul class="grid grid-cols-1 gap-3 xl:grid-cols-2">
        <li v-for="p in publishers" :key="p.id">
          <AppCard :label="p.namespace">
            <div class="flex min-w-0 items-center gap-3">
              <span class="flex size-10 shrink-0 items-center justify-center rounded-xl bg-indigo-50 text-indigo-700 dark:bg-indigo-950 dark:text-indigo-300"><AppIcon name="portal" :size="20" /></span>
              <span class="min-w-0 break-all font-mono text-base font-semibold">{{ p.namespace }}</span>
            </div>
            <div class="mt-3 flex flex-wrap items-center gap-2">
              <AppBadge :tone="p.status === 'approved' ? 'success' : p.status === 'pending' ? 'warn' : 'neutral'">
                {{ statusLabel(p.status) }}
              </AppBadge>
              <AppBadge tone="info" icon="account">{{ p.role }}</AppBadge>
            </div>
            <div class="mt-5 flex flex-col gap-2 border-t border-slate-100 pt-4 dark:border-slate-800">
              <AppButton size="sm" variant="ghost" icon="portal" class="!justify-start" :aria-expanded="open[p.id] === 'members'" @click="toggle(p.id, 'members')">
                {{ t('portal.members') }}
              </AppButton>
              <MemberManager v-if="open[p.id] === 'members'" :publisher-id="p.id" :role="p.role" @changed="refetch" />

              <AppButton size="sm" variant="ghost" icon="key" class="!justify-start" :aria-expanded="open[p.id] === 'keys'" @click="toggle(p.id, 'keys')">
                {{ t('portal.keys') }}
              </AppButton>
              <KeyManager v-if="open[p.id] === 'keys'" :publisher-id="p.id" :role="p.role" :status="p.status" @changed="refetch" />

              <AppButton size="sm" variant="ghost" icon="arrowRight" class="!justify-start" :aria-expanded="open[p.id] === 'transfer'" @click="toggle(p.id, 'transfer')">
                {{ t('portal.transfer') }}
              </AppButton>
              <TransferPanel v-if="open[p.id] === 'transfer'" :publisher-id="p.id" :role="p.role" @changed="refetch" />

              <AppButton size="sm" variant="ghost" icon="package" class="!justify-start" :aria-expanded="open[p.id] === 'releases'" @click="toggle(p.id, 'releases')">
                {{ t('releases.title') }}
              </AppButton>
              <ReleaseList v-if="open[p.id] === 'releases'" :publisher-id="p.id" :refresh-key="releaseRefresh" />
            </div>
            <div class="mt-4 flex flex-wrap items-center justify-between gap-3 border-t border-slate-100 pt-4 dark:border-slate-800">
              <p class="text-xs text-slate-500 dark:text-slate-400">{{ t('portal.notOwnerAdmin') }}</p>
              <AppButton v-if="p.role === 'owner'" size="sm" variant="danger" @click="askDelete(p)">{{ t('portal.deleteNamespace') }}</AppButton>
            </div>
          </AppCard>
        </li>
      </ul>
      <EmptyState v-if="publishers.length === 0" :title="t('portal.emptyDescription')" icon="portal" />
    </StateBlock>

    <ReleaseWizard v-if="publishers.length > 0" :publishers="publishers" @submitted="releaseRefresh++" />
    <ConfirmDialog :open="!!deleteTarget" :title="t('portal.deleteNamespace')" :body="t('portal.deleteConfirm', { namespace: deleteTarget?.namespace ?? '' })" :busy="deleting" :error="deleteError" :prompt-label="t('portal.deletePrompt')" v-model:prompt-value="deleteText" :confirm-match="deleteTarget?.namespace" @confirm="deleteNamespace" @cancel="cancelDelete" />
  </div>
</template>

<script setup lang="ts">
import { computed, reactive, ref, watch } from 'vue'
import { useI18n } from 'vue-i18n'
import { useRoute } from 'vue-router'
import { useQuery, useQueryClient } from '@tanstack/vue-query'
import { ApiError, api } from '../api/client'
import { useUiStore } from '../stores/ui'
import type { MyPublisher } from '../api/types'
import ConfirmDialog from '../components/ConfirmDialog.vue'
import StateBlock from '../components/StateBlock.vue'
import AppIcon from '../components/icons/AppIcon.vue'
import AppBadge from '../components/ui/AppBadge.vue'
import AppButton from '../components/ui/AppButton.vue'
import AppCard from '../components/ui/AppCard.vue'
import AppInput from '../components/ui/AppInput.vue'
import EmptyState from '../components/ui/EmptyState.vue'
import PageHeader from '../components/ui/PageHeader.vue'
import MemberManager from '../components/portal/MemberManager.vue'
import KeyManager from '../components/portal/KeyManager.vue'
import TransferPanel from '../components/portal/TransferPanel.vue'
import ReleaseList from '../components/portal/ReleaseList.vue'
import ReleaseWizard from '../components/portal/ReleaseWizard.vue'

const { t } = useI18n()
const ui = useUiStore()
const route = useRoute()
const queryClient = useQueryClient()

const { data, isPending, error, refetch } = useQuery({
  queryKey: ['portal', 'publishers'],
  queryFn: api.myPublishers,
})
const publishers = computed(() => data.value ?? [])
const open = reactive<Record<string, string>>({})
const releaseRefresh = ref(0)
const deleteTarget = ref<MyPublisher | null>(null)
const deleteText = ref('')
const deleting = ref(false)
const deleteError = ref('')

function askDelete(publisher: MyPublisher) {
  deleteText.value = ''
  deleteError.value = ''
  deleteTarget.value = publisher
}

function cancelDelete() {
  if (!deleting.value) deleteTarget.value = null
}

async function deleteNamespace() {
  const target = deleteTarget.value
  if (!target || deleting.value || deleteText.value !== target.namespace) return
  deleting.value = true
  deleteError.value = ''
  try {
    await api.deletePublisher(target.id)
    delete open[target.id]
    deleteTarget.value = null
    ui.toast(t('portal.deleted'), 'ok')
    await Promise.all([refetch(), queryClient.invalidateQueries({ queryKey: ['my-publishers'] }), queryClient.invalidateQueries({ queryKey: ['admin'] })])
  } catch (e) {
    const message = e instanceof ApiError && e.status === 409 ? 'portal.deleteBlocked'
      : e instanceof ApiError && e.status === 403 ? 'portal.deleteForbidden'
      : e instanceof ApiError && e.status === 404 ? 'portal.deleteMissing'
      : e instanceof ApiError && e.status === 401 ? 'errors.expired'
      : 'errors.actionFailed'
    deleteError.value = t(message)
  } finally {
    deleting.value = false
  }
}

// Google round-trip lands back here with ?stepUp=enroll&publisher=ID: open the
// keys section so KeyManager can consume the one-time grant into memory.
watch(publishers, (list) => {
  const pending = route.query['stepUp'] === 'enroll' ? route.query['publisher'] : undefined
  if (typeof pending === 'string' && list.some((p) => p.id === pending) && !open[pending]) {
    open[pending] = 'keys'
  }
}, { immediate: true })

function toggle(id: string, section: string) {
  if (open[id] === section) delete open[id]
  else open[id] = section
}

function statusLabel(status: string): string {
  if (status === 'pending') return t('portal.statusPending')
  if (status === 'approved') return t('portal.statusApproved')
  if (status === 'suspended') return t('portal.statusSuspended')
  return status
}

const namespace = ref('')
const requesting = ref(false)
const requestError = ref('')
const validNamespace = computed(() => /^[a-z][a-z0-9-]{0,61}[a-z0-9]$/.test(namespace.value.trim()))

async function requestNamespace() {
  if (requesting.value || !validNamespace.value) return
  requesting.value = true
  requestError.value = ''
  try {
    await api.createPublisher(namespace.value.trim())
    namespace.value = ''
    ui.toast(t('portal.requested'), 'ok')
    await refetch()
  } catch (e) {
    if (e instanceof ApiError) {
      if (e.status === 401) requestError.value = t('errors.expired')
      else if (e.status === 409) requestError.value = t('portal.namespaceTaken')
      else if (e.status === 429) requestError.value = t('stepUp.rateLimited')
      else requestError.value = t('errors.actionFailed')
    } else {
      requestError.value = t('errors.actionFailed')
    }
  } finally {
    requesting.value = false
  }
}
</script>
