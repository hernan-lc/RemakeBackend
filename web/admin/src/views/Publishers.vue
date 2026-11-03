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
                  <td class="px-4 py-2.5 text-right">
                    <AppButton
                      v-if="p.status === 'pending'"
                      size="sm"
                      variant="primary"
                      icon="check"
                      :loading="busyId === p.id"
                      @click="askApprove(p.id, p.namespace)"
                    >
                      {{ t('dashboard.approve') }}
                    </AppButton>
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
  </div>
</template>

<script setup lang="ts">
import { computed, ref, watch } from 'vue'
import { useI18n } from 'vue-i18n'
import { useQuery, useQueryClient } from '@tanstack/vue-query'
import { ApiError, api } from '../api/client'
import { useUiStore } from '../stores/ui'
import StateBlock from '../components/StateBlock.vue'
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
const client = useQueryClient()
const status = ref('pending')
const cursor = ref<string | undefined>(undefined)
watch(status, () => { cursor.value = undefined })

const { data, isPending, error, refetch } = useQuery({
  queryKey: computed(() => ['admin', 'publishers', status.value, cursor.value]),
  queryFn: () => api.adminPublishers({ status: status.value || undefined, cursor: cursor.value, limit: 50 }),
})
const items = computed(() => data.value?.items ?? [])
const forbidden = computed(() => error.value instanceof ApiError && error.value.status === 403)
const errorDetail = computed(() =>
  error.value instanceof ApiError && error.value.status !== 403 ? `HTTP ${error.value.status}` : undefined,
)

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
  } catch {
    ui.toast(t('errors.actionFailed'), 'err')
  } finally {
    busyId.value = null
  }
}
</script>
