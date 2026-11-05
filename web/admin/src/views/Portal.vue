<template>
  <div class="flex flex-col gap-5">
    <PageHeader :title="t('portal.title')" />

    <AppCard :label="t('portal.requestTitle')">
      <AppSectionHeader :title="t('portal.requestTitle')" />
      <form class="mt-3 flex flex-col gap-2 sm:flex-row sm:items-end" @submit.prevent="requestNamespace">
        <AppInput v-model="namespace" :label="t('portal.namespace')" required :maxlength="63" class="min-w-0 flex-1" />
        <AppButton type="submit" size="sm" variant="primary" icon="plus" :loading="requesting" :disabled="requesting || !validNamespace">
          {{ t('portal.requestButton') }}
        </AppButton>
      </form>
      <p class="mt-2 text-xs text-slate-500 dark:text-slate-400">{{ t('portal.namespaceHint') }}</p>
      <p v-if="requestError" role="alert" class="mt-2 text-sm text-red-700 dark:text-red-300">{{ requestError }}</p>
    </AppCard>

    <StateBlock :loading="isPending" :error="!!error" :retry="true" @retry="refetch">
      <div class="mb-3">
        <AppSectionHeader :title="t('portal.yourNamespaces')">
          <template #actions><AppBadge tone="neutral">{{ publishers.length }}</AppBadge></template>
        </AppSectionHeader>
      </div>
      <ul class="grid grid-cols-1 gap-3">
        <li v-for="p in publishers" :key="p.id">
          <AppCard :label="p.namespace">
            <div class="flex min-w-0 flex-wrap items-center gap-2">
              <h2 class="mr-2 min-w-0 break-all font-mono text-base font-semibold">{{ p.namespace }}</h2>
              <AppBadge :tone="p.status === 'approved' ? 'success' : p.status === 'pending' ? 'warn' : 'neutral'">
                {{ statusLabel(p.status) }}
              </AppBadge>
              <AppBadge tone="neutral">{{ p.role }}</AppBadge>
            </div>
            <div class="mt-3">
              <AppActionTabs :model-value="open[p.id] ?? ''" :items="namespaceTabs" :label="p.namespace" @update:model-value="setOpen(p.id, $event)" />
            </div>
            <div v-if="open[p.id]" class="mt-3 rounded-lg border border-slate-200 bg-slate-50/50 p-3 dark:border-slate-800 dark:bg-slate-800/30">
              <MemberManager v-if="open[p.id] === 'members'" :publisher-id="p.id" :role="p.role" @changed="refetch" />
              <KeyManager v-if="open[p.id] === 'keys'" :publisher-id="p.id" :role="p.role" :status="p.status" @changed="refetch" />
              <TransferPanel v-if="open[p.id] === 'transfer'" :publisher-id="p.id" :role="p.role" @changed="refetch" />
              <ReleaseList v-if="open[p.id] === 'releases'" :publisher-id="p.id" :refresh-key="releaseRefresh" />
            </div>
            <div v-if="p.role === 'owner'" class="mt-3 flex justify-end border-t border-slate-100 pt-3 dark:border-slate-800">
              <AppButton size="sm" variant="danger" icon="trash" @click="askDelete(p)">{{ t('portal.deleteNamespace') }}</AppButton>
            </div>
          </AppCard>
        </li>
      </ul>
      <EmptyState v-if="publishers.length === 0" :title="t('portal.emptyDescription')" icon="portal" />
    </StateBlock>

    <ReleaseWizard v-if="publishers.some((p) => p.status === 'approved' && canPublish(p.role))" :publishers="publishers" @submitted="releaseRefresh++" />
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
import { canPublish } from '../api/types'
import type { MyPublisher } from '../api/types'
import ConfirmDialog from '../components/ConfirmDialog.vue'
import StateBlock from '../components/StateBlock.vue'
import AppBadge from '../components/ui/AppBadge.vue'
import AppButton from '../components/ui/AppButton.vue'
import AppCard from '../components/ui/AppCard.vue'
import AppSectionHeader from '../components/ui/AppSectionHeader.vue'
import AppInput from '../components/ui/AppInput.vue'
import AppActionTabs from '../components/ui/AppActionTabs.vue'
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

const namespaceTabs = computed(() => [
  { value: 'members', label: t('portal.members'), icon: 'portal' as const },
  { value: 'keys', label: t('portal.keys'), icon: 'key' as const },
  { value: 'transfer', label: t('portal.transfer'), icon: 'arrowRight' as const },
  { value: 'releases', label: t('releases.title'), icon: 'package' as const },
])
function setOpen(id: string, section: string) {
  if (!section) delete open[id]
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
