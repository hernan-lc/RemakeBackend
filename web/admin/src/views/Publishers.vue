<template>
  <div class="flex flex-col gap-4">
    <div class="flex flex-wrap items-center gap-3">
      <h1 class="text-xl font-bold">{{ t('publishers.title') }}</h1>
      <label class="ml-auto flex items-center gap-2 text-sm">
        {{ t('publishers.filter') }}
        <select v-model="status" class="rounded border border-slate-300 px-2 py-1.5">
          <option value="pending">{{ t('publishers.pending') }}</option>
          <option value="approved">{{ t('publishers.approved') }}</option>
          <option value="suspended">{{ t('publishers.suspended') }}</option>
          <option value="">{{ t('publishers.all') }}</option>
        </select>
      </label>
    </div>
    <StateBlock :loading="isPending" :error="!!error" :detail="errorDetail" :retry="true" @retry="refetch">
      <div v-if="forbidden" class="rounded border border-amber-200 bg-amber-50 p-4 text-sm text-amber-800" role="alert">
        {{ t('errors.notAdmin') }}
      </div>
      <template v-else>
        <table class="w-full overflow-hidden rounded border border-slate-200 bg-white text-left text-sm">
          <thead>
            <tr class="border-b border-slate-200 bg-slate-50">
              <th scope="col" class="px-3 py-2">{{ t('publishers.namespace') }}</th>
              <th scope="col" class="px-3 py-2">{{ t('publishers.status') }}</th>
              <th scope="col" class="px-3 py-2">{{ t('publishers.created') }}</th>
              <th scope="col" class="px-3 py-2">{{ t('publishers.actions') }}</th>
            </tr>
          </thead>
          <tbody>
            <tr v-for="p in items" :key="p.id" class="border-b border-slate-100 last:border-0">
              <td class="px-3 py-2 font-mono">{{ p.namespace }}</td>
              <td class="px-3 py-2">{{ p.status }}</td>
              <td class="px-3 py-2 tabular-nums">{{ p.createdAt }}</td>
              <td class="px-3 py-2">
                <button
                  v-if="p.status === 'pending'"
                  type="button"
                  :disabled="busyId === p.id"
                  class="rounded bg-slate-900 px-3 py-1 text-sm font-medium text-white hover:bg-slate-700 disabled:opacity-50"
                  @click="askApprove(p.id, p.namespace)"
                >
                  {{ t('dashboard.approve') }}
                </button>
              </td>
            </tr>
          </tbody>
        </table>
        <p v-if="items.length === 0" class="mt-3 text-sm text-slate-500">{{ t('states.empty') }}</p>
        <button
          v-if="data?.nextCursor"
          type="button"
          class="mt-3 rounded border border-slate-300 px-3 py-1.5 text-sm font-medium hover:bg-slate-50"
          @click="cursor = data.nextCursor"
        >
          {{ t('states.next') }}
        </button>
      </template>
    </StateBlock>
    <ConfirmDialog :open="confirmOpen" :title="t('dashboard.approve')" :body="t('dashboard.approveConfirm')" :busy="busyId !== null" @confirm="doApprove" @cancel="confirmOpen = false" />
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
