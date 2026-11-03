<template>
  <div class="flex flex-col gap-4">
    <h1 class="text-xl font-bold">{{ t('moderation.title') }}</h1>
    <p class="text-sm text-slate-600">{{ t('moderation.conflictNote') }}</p>
    <StateBlock :loading="isPending" :error="!!error" :detail="errorDetail" :retry="true" @retry="refetch">
      <template v-if="forbidden">
        <div role="alert" class="rounded border border-amber-200 bg-amber-50 p-4 text-sm text-amber-800">
          {{ t('errors.notAdmin') }}
        </div>
      </template>
      <ul v-else class="flex flex-col gap-3">
        <li v-for="c in items" :key="c.id" class="rounded border border-slate-200 bg-white p-4">
          <div class="flex flex-wrap items-baseline gap-x-3">
            <span class="font-mono text-sm font-semibold">{{ c.namespace }}/{{ c.pluginId }}@{{ c.version }}</span>
            <span class="text-xs text-slate-500">{{ c.target }} · {{ (c.size / 1024).toFixed(1) }} KiB</span>
            <span v-if="c.yanked" class="rounded bg-red-100 px-2 py-0.5 text-xs font-medium text-red-800">yanked</span>
            <span v-if="c.advisory" class="rounded bg-amber-100 px-2 py-0.5 text-xs font-medium text-amber-800">{{ c.advisory.severity }}</span>
          </div>
          <p class="mt-1 break-all font-mono text-xs text-slate-500">sha256:{{ c.sha256 }} · key {{ c.keyId }}</p>
          <div class="mt-3 flex flex-wrap gap-2">
            <button type="button" class="rounded border border-slate-300 px-3 py-1 text-sm font-medium hover:bg-slate-50" @click="inspect(c.id)">
              {{ t('moderation.inspect') }}
            </button>
            <button type="button" :disabled="c.yanked" class="rounded bg-slate-900 px-3 py-1 text-sm font-medium text-white hover:bg-slate-700 disabled:opacity-40" @click="askDecide(c.id, 'approve')">
              {{ t('moderation.approve') }}
            </button>
            <button type="button" :disabled="c.yanked" class="rounded border border-slate-300 px-3 py-1 text-sm font-medium hover:bg-slate-50 disabled:opacity-40" @click="askDecide(c.id, 'reject')">
              {{ t('moderation.reject') }}
            </button>
            <button type="button" :disabled="c.yanked" class="rounded border border-red-300 px-3 py-1 text-sm font-medium text-red-700 hover:bg-red-50 disabled:opacity-40" @click="askYank(c.id)">
              {{ t('moderation.yank') }}
            </button>
            <button type="button" class="rounded border border-slate-300 px-3 py-1 text-sm font-medium hover:bg-slate-50" @click="openAdvisory(c.id)">
              {{ t('moderation.advisory') }}
            </button>
          </div>
          <p v-if="links[c.id]" class="mt-2 break-all text-xs">
            {{ t('moderation.inspectionReady') }}
            <a :href="links[c.id]" class="font-medium text-slate-900 underline" target="_blank" rel="noreferrer noopener">{{ links[c.id] }}</a>
          </p>
          <form v-if="advisoryFor === c.id" class="mt-3 flex flex-col gap-2 rounded border border-slate-200 p-3" @submit.prevent="saveAdvisory(c.id)">
            <label class="text-sm">{{ t('moderation.severity') }}
              <select v-model="adv.severity" class="ml-2 rounded border border-slate-300 px-2 py-1">
                <option>low</option><option>moderate</option><option>high</option><option>critical</option>
              </select>
            </label>
            <label class="text-sm">{{ t('moderation.message') }}
              <input v-model="adv.message" required maxlength="1024" class="ml-2 w-full max-w-md rounded border border-slate-300 px-2 py-1" />
            </label>
            <label class="text-sm">{{ t('moderation.url') }}
              <input v-model="adv.url" type="url" class="ml-2 w-full max-w-md rounded border border-slate-300 px-2 py-1" />
            </label>
            <div class="flex gap-2">
              <button type="submit" class="rounded bg-slate-900 px-3 py-1 text-sm font-medium text-white">{{ t('states.confirm') }}</button>
              <button type="button" class="rounded border border-slate-300 px-3 py-1 text-sm" @click="advisoryFor = null">{{ t('states.cancel') }}</button>
            </div>
          </form>
        </li>
      </ul>
      <p v-if="!forbidden && items.length === 0" class="mt-3 text-sm text-slate-500">{{ t('states.empty') }}</p>
      <button v-if="data?.nextCursor" type="button" class="mt-3 rounded border border-slate-300 px-3 py-1.5 text-sm font-medium hover:bg-slate-50" @click="after = data.nextCursor ?? undefined">
        {{ t('states.next') }}
      </button>
    </StateBlock>
    <ConfirmDialog :open="confirmOpen" :title="confirmTitle" :body="confirmBody" :busy="busy" @confirm="doConfirm" @cancel="confirmOpen = false" />
  </div>
</template>

<script setup lang="ts">
import { computed, reactive, ref } from 'vue'
import { useI18n } from 'vue-i18n'
import { useQuery, useQueryClient } from '@tanstack/vue-query'
import { ApiError, api } from '../api/client'
import { useUiStore } from '../stores/ui'
import StateBlock from '../components/StateBlock.vue'
import ConfirmDialog from '../components/ConfirmDialog.vue'

const { t } = useI18n()
const ui = useUiStore()
const client = useQueryClient()
const after = ref<string | undefined>(undefined)

const { data, isPending, error, refetch } = useQuery({
  queryKey: computed(() => ['moderation', 'queue', after.value]),
  queryFn: () => api.moderationQueue(after.value),
})
const items = computed(() => data.value?.items ?? [])
const forbidden = computed(() => error.value instanceof ApiError && error.value.status === 403)
const errorDetail = computed(() =>
  error.value instanceof ApiError && error.value.status !== 403 ? `HTTP ${error.value.status}` : undefined,
)

const links = reactive<Record<string, string>>({})
const confirmOpen = ref(false)
const confirmTitle = ref('')
const confirmBody = ref('')
const busy = ref(false)
let confirmAction: (() => Promise<void>) | null = null
const advisoryFor = ref<string | null>(null)
const adv = reactive({ severity: 'high', message: '', url: '' })
const decideChoice = reactive({ id: '', decision: 'approve' as 'approve' | 'reject' })

async function inspect(id: string) {
  try {
    const r = await api.inspectRelease(id)
    links[id] = r.url
  } catch {
    ui.toast(t('errors.actionFailed'), 'err')
  }
}

function askDecide(id: string, decision: 'approve' | 'reject') {
  decideChoice.id = id
  decideChoice.decision = decision
  confirmTitle.value = decision === 'approve' ? t('moderation.approve') : t('moderation.reject')
  confirmBody.value = t('moderation.decideConfirm')
  confirmAction = async () => {
    await api.decideRelease(id, decision, decision === 'approve' ? 'policy_reviewed' : 'policy')
    ui.toast(t('moderation.decided'), 'ok')
  }
  confirmOpen.value = true
}

function askYank(id: string) {
  const reason = window.prompt(t('moderation.yankReason'))
  if (!reason) return
  confirmTitle.value = t('moderation.yank')
  confirmBody.value = t('moderation.yankConfirm')
  confirmAction = async () => {
    await api.yankRelease(id, reason)
    ui.toast(t('moderation.yanked'), 'ok')
  }
  confirmOpen.value = true
}

function openAdvisory(id: string) {
  advisoryFor.value = id
  adv.severity = 'high'
  adv.message = ''
  adv.url = ''
}

async function saveAdvisory(id: string) {
  try {
    await api.recordAdvisory(id, adv.severity, adv.message, adv.url || undefined)
    ui.toast(t('moderation.advisorySaved'), 'ok')
    advisoryFor.value = null
    await client.invalidateQueries({ queryKey: ['moderation'] })
  } catch {
    ui.toast(t('errors.actionFailed'), 'err')
  }
}

async function doConfirm() {
  if (!confirmAction) return
  busy.value = true
  try {
    await confirmAction()
    confirmOpen.value = false
    await client.invalidateQueries({ queryKey: ['moderation'] })
    await client.invalidateQueries({ queryKey: ['admin'] })
    await refetch()
  } catch (e) {
    ui.toast(t('errors.actionFailed'), 'err')
    if (e instanceof ApiError && (e.status === 403 || e.status === 409)) confirmOpen.value = false
  } finally {
    busy.value = false
  }
}
</script>
