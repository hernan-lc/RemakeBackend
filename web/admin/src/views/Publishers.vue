<template>
  <div class="flex flex-col gap-4">
    <PageHeader :title="t('publishers.title')" :description="copy.description" />
    <AppToolbar v-model="search" :search-label="t('publishers.searchPage')" :refresh-label="t('states.refresh')" @refresh="refetch">
      <template #filters>
        <AppSelect v-model="status" :label="t('publishers.filter')" class="[&>span:first-child]:sr-only">
          <option value="pending">{{ t('publishers.pending') }}</option>
          <option value="approved">{{ t('publishers.approved') }}</option>
          <option value="suspended">{{ t('publishers.suspended') }}</option>
          <option value="">{{ t('publishers.all') }}</option>
        </AppSelect>
      </template>
    </AppToolbar>
    <StateBlock :loading="isPending" :error="!!error && !forbidden" :detail="errorDetail" :retry="true" @retry="refetch">
      <div v-if="forbidden" class="flex items-start gap-2 rounded-lg border border-amber-200 bg-amber-50 p-4 text-sm text-amber-800 dark:border-amber-900 dark:bg-amber-950 dark:text-amber-200" role="alert">
        <AppIcon name="alert" :size="17" class="mt-0.5 shrink-0" aria-hidden="true" />
        {{ t('errors.notAdmin') }}
      </div>
      <template v-else>
        <div class="mb-4 flex flex-wrap items-center justify-between gap-2">
          <h2 class="text-sm font-semibold">{{ status ? statusLabel(status) : t('publishers.all') }}</h2>
          <span class="text-xs text-slate-600 dark:text-slate-300">{{ visibleItems.length }} {{ copy.loaded }}</span>
        </div>
        <p v-if="visibleItems.some(p => isMember(p.id))" class="mb-3 flex items-start gap-2 text-xs text-amber-700 dark:text-amber-300">
          <AppIcon name="info" :size="16" class="shrink-0" aria-hidden="true" />
          {{ t(temporaryEnabled ? 'temporaryApproval.overrideNotice' : 'publishers.independentReviewer') }}
        </p>
        <AppCard v-if="visibleItems.length > 0 || membershipsError" :padded="false" label="publisher-requests">
          <p v-if="membershipsError" class="p-4 text-sm text-amber-700 dark:text-amber-300" role="alert">{{ t('publishers.membershipsUnavailable') }} <button class="underline" @click="refetchMemberships()">{{ t('states.retry') }}</button></p>
          <div class="scroll-thin overflow-x-auto">
            <table class="block w-full text-left text-sm md:table md:table-fixed">
              <thead class="sr-only md:not-sr-only md:table-header-group">
                <tr class="border-b border-slate-200 bg-slate-50 text-xs font-semibold uppercase tracking-wide text-slate-500 dark:border-slate-800 dark:bg-slate-800 dark:text-slate-400">
                  <th scope="col" class="px-4 py-2.5 font-semibold">{{ t('publishers.namespace') }}</th>
                  <th scope="col" class="px-4 py-2.5 font-semibold">{{ t('publishers.status') }}</th>
                  <th scope="col" class="px-4 py-2.5 font-semibold">{{ t('publishers.created') }}</th>
                  <th scope="col" class="px-4 py-2.5 font-semibold md:w-[38%]"><span class="sr-only">{{ t('publishers.actions') }}</span></th>
                </tr>
              </thead>
              <tbody class="block divide-y divide-slate-100 md:table-row-group dark:divide-slate-800">
                <tr v-for="p in visibleItems" :key="p.id" class="block py-3 transition-colors hover:bg-slate-50 md:table-row md:py-0 dark:hover:bg-slate-800/40">
                  <td class="block break-all px-4 py-1 font-mono font-semibold md:table-cell md:py-2.5">{{ p.namespace }}</td>
                  <td class="block px-4 py-1 md:table-cell md:py-2.5"><span class="mr-2 text-xs text-slate-600 dark:text-slate-300 md:hidden">{{ t('publishers.status') }}:</span><AppBadge :tone="statusTone(p.status)">{{ statusLabel(p.status) }}</AppBadge></td>
                  <td class="block break-words px-4 py-1 text-slate-600 tabular-nums md:table-cell md:py-2.5 dark:text-slate-400"><span class="mr-2 text-xs md:hidden">{{ t('publishers.created') }}:</span><span>{{ formatDateTime(p.createdAt, locale) }}</span></td>
                  <td class="block px-4 py-2 md:table-cell md:py-2.5"><div class="flex flex-wrap items-center gap-2 md:justify-end md:text-right">
                    <AppButton
                      v-if="p.status === 'pending'"
                      size="sm"
                      variant="primary"
                      icon="check"
                      :loading="busyId === p.id"
                      :disabled="!memberships || isMember(p.id)"
                      @click="askApprove(p.id, p.namespace)"
                    >
                      {{ t('dashboard.approve') }}
                    </AppButton>
                    <AppButton v-if="p.status === 'pending' && isMember(p.id) && temporaryEnabled" size="sm" variant="secondary" @click="openTemporary(p.id, p.namespace, null)">{{ t('temporaryApproval.button') }}</AppButton>
                    <AppButton size="sm" variant="ghost" icon="key" :aria-expanded="!!keys[p.id]" :disabled="!memberships || (isMember(p.id) && !temporaryEnabled)" @click="toggleKeys(p.id)">
                      {{ t('publishers.keys') }}
                    </AppButton>
                    <p v-if="keys[p.id] && pendingKeys(p.id).length === 0" class="text-xs text-slate-500 dark:text-slate-400">{{ t('publishers.noPendingKeys') }}</p>
                    <ul v-if="keys[p.id]" class="mt-2 flex w-full min-w-0 flex-col gap-3 text-left">
                      <li v-for="k in pendingKeys(p.id)" :key="k.id" class="flex flex-col items-start gap-2 rounded-xl border border-slate-200 bg-slate-50 px-4 py-3 dark:border-slate-700 dark:bg-slate-800">
                        <span class="break-all font-mono text-xs font-semibold">{{ k.keyId }}</span>
                        <span class="text-xs text-slate-500 dark:text-slate-400">{{ t('publishers.fingerprint') }} (SHA-256)</span>
                        <code class="break-all font-mono text-xs">{{ k.fingerprint }}</code>
                        <p class="text-xs text-slate-500 dark:text-slate-400">{{ t('publishers.compareFingerprint') }}</p>
                        <AppButton
                          v-if="!isMember(p.id)"
                          size="sm"
                          variant="secondary"
                          icon="check"
                          :loading="keyBusyId === k.id"
                          @click="askApproveKey(p.id, k.id)"
                        >
                          {{ t('publishers.approveKey') }}
                        </AppButton>
                        <AppButton v-if="isMember(p.id) && temporaryEnabled" size="sm" variant="secondary" @click="openTemporary(p.id, p.namespace, k.id)">{{ t('temporaryApproval.button') }}</AppButton>
                      </li>
                    </ul>
                  </div></td>
                </tr>
              </tbody>
            </table>
          </div>
        </AppCard>
        <EmptyState v-if="visibleItems.length === 0" :title="search ? t('states.noMatches') : t('states.empty')" icon="publishers" />
        <div v-if="data?.nextCursor" class="flex justify-center">
          <AppButton variant="secondary" icon="chevronDown" @click="cursor = data.nextCursor">
            {{ t('states.next') }}
          </AppButton>
        </div>
      </template>
    </StateBlock>
    <ConfirmDialog :danger="false" :open="confirmOpen" :title="t('dashboard.approve')" :body="`${pendingNs} — ${t('dashboard.approveConfirm')}`" :busy="busyId !== null" @confirm="doApprove" @cancel="confirmOpen = false" />
    <TemporaryApprovalDialog :open="!!temporaryTarget" :publisher-id="temporaryTarget?.publisherId ?? ''" :namespace="temporaryTarget?.namespace ?? ''" :key-id="temporaryTarget?.keyId ?? null" @cancelled="temporaryTarget = null" @approved="temporaryApproved" />
    <StepUpDialog
      :open="keyDialogOpen"
      action="publisher.key.approve"
      :action-label="t('publishers.approveKey')"
      :publisher-id="pendingKey?.publisherId ?? ''"
      :key-id="pendingKey?.keyId ?? null"
      :return-to="keyReturnTo"
      @granted="doApproveKey"
      @cancelled="keyDialogOpen = false"
    />
  </div>
</template>

<script setup lang="ts">
import { computed, onMounted, reactive, ref, watch } from 'vue'
import { useI18n } from 'vue-i18n'
import { useRoute, useRouter } from 'vue-router'
import { useQuery, useQueryClient } from '@tanstack/vue-query'
import { ApiError, api } from '../api/client'
import { parseStepUpQuery } from '../api/auth'
import { useUiStore } from '../stores/ui'
import type { PublisherKey } from '../api/types'
import StateBlock from '../components/StateBlock.vue'
import StepUpDialog from '../components/StepUpDialog.vue'
import ConfirmDialog from '../components/ConfirmDialog.vue'
import TemporaryApprovalDialog from '../components/TemporaryApprovalDialog.vue'
import AppIcon from '../components/icons/AppIcon.vue'
import AppBadge from '../components/ui/AppBadge.vue'
import AppButton from '../components/ui/AppButton.vue'
import AppCard from '../components/ui/AppCard.vue'
import AppSelect from '../components/ui/AppSelect.vue'
import AppToolbar from '../components/ui/AppToolbar.vue'
import { formatDateTime } from '../utils/format'
import EmptyState from '../components/ui/EmptyState.vue'
import PageHeader from '../components/ui/PageHeader.vue'

const { t, locale } = useI18n()
const copy = computed(() => locale.value === 'es' ? { description: 'Revisa las solicitudes de namespaces y sus claves de firma con un administrador independiente.', loaded: 'solicitudes en esta página' } : { description: 'Review namespace requests and signing keys with an independent administrator.', loaded: 'requests on this page' })
function statusLabel(value: string) { return ['pending', 'approved', 'suspended'].includes(value) ? t(`publishers.${value}`) : value }
const ui = useUiStore()
const route = useRoute()
const router = useRouter()
const client = useQueryClient()
const status = ref('pending')
const search = ref('')
const cursor = ref<string | undefined>(undefined)
watch(status, () => { cursor.value = undefined })

const { data, isPending, error, refetch } = useQuery({
  queryKey: computed(() => ['admin', 'publishers', status.value, cursor.value]),
  queryFn: () => api.adminPublishers({ status: status.value || undefined, cursor: cursor.value, limit: 50 }),
})
const items = computed(() => data.value?.items ?? [])
const visibleItems = computed(() => {
  const query = search.value.trim().toLowerCase()
  return query ? items.value.filter((p) => [p.namespace, p.status].some((text) => text.toLowerCase().includes(query))) : items.value
})
const { data: temporaryAvailability } = useQuery({
  queryKey: ['admin', 'temporary-approval-availability'],
  queryFn: api.temporaryApprovalAvailability,
  retry: false,
})
const temporaryEnabled = computed(() => temporaryAvailability.value?.enabled === true)
const temporaryTarget = ref<{ publisherId: string; namespace: string; keyId: string | null } | null>(null)
function openTemporary(publisherId: string, namespace: string, keyId: string | null) {
  temporaryTarget.value = { publisherId, namespace, keyId }
}
async function temporaryApproved() {
  if (temporaryTarget.value) {
    delete keys[temporaryTarget.value.publisherId]
    if (!temporaryTarget.value.keyId) status.value = 'approved'
  }
  temporaryTarget.value = null
  ui.toast(t('publishers.approvedMsg'), 'ok')
  await client.invalidateQueries({ queryKey: ['admin'] })
  await refetch()
}
const { data: memberships, error: membershipsError, refetch: refetchMemberships } = useQuery({
  queryKey: ['my-publishers', 'review-eligibility'],
  queryFn: api.myPublishers,
})
function isMember(id: string): boolean {
  return memberships.value?.some((publisher) => publisher.id === id) ?? false
}
const forbidden = computed(() => error.value instanceof ApiError && error.value.status === 403)
const errorDetail = computed(() =>
  error.value instanceof ApiError && error.value.status !== 403 ? `HTTP ${error.value.status}` : undefined,
)
const keys = reactive<Record<string, PublisherKey[]>>({})

function pendingKeys(publisherId: string): PublisherKey[] {
  return (keys[publisherId] ?? []).filter((k) => k.status === 'pending')
}

async function toggleKeys(publisherId: string) {
  if (keys[publisherId]) {
    delete keys[publisherId]
    return
  }
  try {
    keys[publisherId] = isMember(publisherId) ? await api.publisherKeys(publisherId) : await api.adminPublisherKeys(publisherId)
  } catch {
    ui.toast(t('errors.loadFailed'), 'err')
  }
}

function statusTone(s: string): 'success' | 'warn' | 'danger' | 'neutral' {
  if (s === 'approved') return 'success'
  if (s === 'pending') return 'warn'
  if (s === 'suspended') return 'danger'
  return 'neutral'
}

const confirmOpen = ref(false)
const pendingId = ref<string | null>(null)
const pendingNs = ref('')
const busyId = ref<string | null>(null)

function askApprove(id: string, ns: string) {
  pendingId.value = id
  pendingNs.value = ns
  confirmOpen.value = true
}

async function doApprove() {
  if (!pendingId.value) return
  busyId.value = pendingId.value
  try {
    await api.approvePublisher(pendingId.value)
    ui.toast(t('publishers.approvedMsg'), 'ok')
    confirmOpen.value = false
    await client.invalidateQueries({ queryKey: ['admin'] })
    await refetch()
  } catch (e) {
    if (e instanceof ApiError && e.status === 403) {
      confirmOpen.value = false
      await refetchMemberships()
      ui.toast(t('publishers.approvalForbidden'), 'err')
    } else {
      ui.toast(t('errors.actionFailed'), 'err')
    }
  } finally {
    busyId.value = null
  }
}

const keyDialogOpen = ref(false)
const pendingKey = ref<{ publisherId: string; keyId: string } | null>(null)
const keyBusyId = ref<string | null>(null)
const keyReturnTo = ref('/app/publishers')

function askApproveKey(publisherId: string, keyId: string) {
  pendingKey.value = { publisherId, keyId }
  keyReturnTo.value = `/app/publishers?stepUp=approve&publisher=${publisherId}&key=${keyId}`
  keyDialogOpen.value = true
}

async function doApproveKey(grantId: string) {
  const target = pendingKey.value
  keyDialogOpen.value = false
  if (!target) return
  keyBusyId.value = target.keyId
  try {
    await api.approvePublisherKey(target.publisherId, target.keyId, grantId)
    ui.toast(t('publishers.approvedKeyMsg'), 'ok')
    pendingKey.value = null
    delete keys[target.publisherId]
    await refetch()
  } catch (e) {
    if (e instanceof ApiError && e.status === 401) {
      await router.replace({ path: '/login', query: { next: '/app/publishers' } })
    } else if (e instanceof ApiError && e.status === 403 && e.code === 'step_up_expired') {
      keyReturnTo.value = `/app/publishers?stepUp=approve&publisher=${target.publisherId}&key=${target.keyId}`
      keyDialogOpen.value = true
      ui.toast(t('stepUp.expired'), 'err')
    } else {
      ui.toast(t('errors.actionFailed'), 'err')
    }
  } finally {
    keyBusyId.value = null
  }
}

// Google round-trip: the callback redirects here with the one-time grant (or
// an opaque error) in the query. The grant is consumed into memory and the
// query is stripped immediately.
const resumed = ref(false)
async function resumeFromQuery() {
  if (resumed.value) return
  if (route.query['stepUp'] !== 'approve') return
  const publisherId = route.query['publisher']
  const keyId = route.query['key']
  if (typeof publisherId !== 'string' || typeof keyId !== 'string') return
  resumed.value = true
  const outcome = parseStepUpQuery(route.query as Record<string, unknown>)
  const clean = { ...route.query }
  delete clean['stepUp']
  delete clean['publisher']
  delete clean['key']
  delete clean['stepUpGrant']
  delete clean['stepUpError']
  await router.replace({ path: route.path, query: clean })
  if (outcome && 'grantId' in outcome) {
    pendingKey.value = { publisherId, keyId }
    await doApproveKey(outcome.grantId)
  } else if (outcome && 'error' in outcome) {
    ui.toast(t('stepUp.failed'), 'err')
  }
}

onMounted(() => {
  void resumeFromQuery()
})
watch(items, () => {
  void resumeFromQuery()
})
</script>
