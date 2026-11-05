<template>
  <div class="flex flex-col gap-4">
    <PageHeader :title="t('publishers.title')" icon="publishers">
      <template #actions>
        <AppSelect v-model="status" :label="t('publishers.filter')">
          <option value="pending">{{ t('publishers.pending') }}</option>
          <option value="approved">{{ t('publishers.approved') }}</option>
          <option value="suspended">{{ t('publishers.suspended') }}</option>
          <option value="">{{ t('publishers.all') }}</option>
        </AppSelect>
        <AppButton size="sm" variant="secondary" icon="refresh" icon-only :label="t('states.refresh')" :title="t('states.refresh')" @click="refetch" />
      </template>
    </PageHeader>
    <StateBlock :loading="isPending" :error="!!error" :detail="errorDetail" :retry="true" @retry="refetch">
      <div v-if="forbidden" class="flex items-start gap-2 rounded-lg border border-amber-200 bg-amber-50 p-4 text-sm text-amber-800 dark:border-amber-900 dark:bg-amber-950 dark:text-amber-200" role="alert">
        <AppIcon name="alert" :size="17" class="mt-0.5 shrink-0" aria-hidden="true" />
        {{ t('errors.notAdmin') }}
      </div>
      <template v-else>
        <AppCard :padded="false" label="publisher-requests">
          <p v-if="membershipsError" class="p-4 text-sm text-amber-700 dark:text-amber-300" role="alert">{{ t('publishers.membershipsUnavailable') }} <button class="underline" @click="refetchMemberships()">{{ t('states.retry') }}</button></p>
          <div class="scroll-thin overflow-x-auto">
            <table class="w-full min-w-[36rem] text-left text-sm">
              <thead>
                <tr class="border-b border-slate-200 bg-slate-50 dark:border-slate-700 dark:bg-slate-800">
                  <th scope="col" class="px-4 py-2.5 font-semibold">{{ t('publishers.namespace') }}</th>
                  <th scope="col" class="px-4 py-2.5 font-semibold">{{ t('publishers.status') }}</th>
                  <th scope="col" class="px-4 py-2.5 font-semibold">{{ t('publishers.created') }}</th>
                  <th scope="col" class="px-4 py-2.5 font-semibold"><span class="sr-only">{{ t('publishers.actions') }}</span></th>
                </tr>
              </thead>
              <tbody class="divide-y divide-slate-100 dark:divide-slate-800">
                <tr v-for="p in items" :key="p.id">
                  <td class="px-4 py-2.5 font-mono">{{ p.namespace }}</td>
                  <td class="px-4 py-2.5"><AppBadge :tone="statusTone(p.status)">{{ p.status }}</AppBadge></td>
                  <td class="whitespace-nowrap px-4 py-2.5 text-slate-600 tabular-nums dark:text-slate-400">{{ p.createdAt }}</td>
                  <td class="flex flex-col items-end gap-2 px-4 py-2.5 text-right">
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
                    <p v-if="isMember(p.id)" class="max-w-80 text-xs text-amber-700 dark:text-amber-300">{{ t('publishers.independentReviewer') }}</p>
                    <AppButton size="sm" variant="ghost" :disabled="!memberships || isMember(p.id)" @click="toggleKeys(p.id)">
                      {{ t('publishers.keys') }}
                    </AppButton>
                    <ul v-if="keys[p.id]" class="flex w-full flex-col gap-2 text-left">
                      <li v-for="k in pendingKeys(p.id)" :key="k.id" class="flex flex-wrap items-center gap-2 rounded-md border border-slate-200 px-2 py-1.5 dark:border-slate-700">
                        <span class="min-w-0 flex-1 truncate font-mono text-xs">{{ k.keyId }}</span>
                        <AppButton
                          size="sm"
                          variant="secondary"
                          icon="check"
                          :loading="keyBusyId === k.id"
                          @click="askApproveKey(p.id, k.id)"
                        >
                          {{ t('publishers.approveKey') }}
                        </AppButton>
                      </li>
                    </ul>
                  </td>
                </tr>
              </tbody>
            </table>
          </div>
        </AppCard>
        <EmptyState v-if="items.length === 0" :title="t('states.empty')" icon="publishers" />
        <div v-if="data?.nextCursor" class="flex justify-center">
          <AppButton variant="secondary" icon="chevronDown" @click="cursor = data.nextCursor">
            {{ t('states.next') }}
          </AppButton>
        </div>
      </template>
    </StateBlock>
    <ConfirmDialog :danger="false" :open="confirmOpen" :title="t('dashboard.approve')" :body="t('dashboard.approveConfirm')" :busy="busyId !== null" @confirm="doApprove" @cancel="confirmOpen = false" />
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
import AppIcon from '../components/icons/AppIcon.vue'
import AppBadge from '../components/ui/AppBadge.vue'
import AppButton from '../components/ui/AppButton.vue'
import AppCard from '../components/ui/AppCard.vue'
import AppSelect from '../components/ui/AppSelect.vue'
import EmptyState from '../components/ui/EmptyState.vue'
import PageHeader from '../components/ui/PageHeader.vue'

const { t } = useI18n()
const ui = useUiStore()
const route = useRoute()
const router = useRouter()
const client = useQueryClient()
const status = ref('pending')
const cursor = ref<string | undefined>(undefined)
watch(status, () => { cursor.value = undefined })

const { data, isPending, error, refetch } = useQuery({
  queryKey: computed(() => ['admin', 'publishers', status.value, cursor.value]),
  queryFn: () => api.adminPublishers({ status: status.value || undefined, cursor: cursor.value, limit: 50 }),
})
const items = computed(() => data.value?.items ?? [])
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
    keys[publisherId] = await api.adminPublisherKeys(publisherId)
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
