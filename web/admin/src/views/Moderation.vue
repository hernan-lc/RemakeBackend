<template>
  <div class="flex flex-col gap-4">
    <PageHeader :title="t('moderation.title')" :description="t('moderation.conflictNote')" icon="moderation">
      <template #actions>
        <AppButton size="sm" variant="secondary" icon="refresh" icon-only :label="t('states.refresh')" :title="t('states.refresh')" @click="refetch" />
      </template>
    </PageHeader>
    <div class="flex flex-col items-start gap-2">
      <AppSearch v-model="search" :label="t('moderation.searchPage')" />
      <p class="text-xs text-slate-500 dark:text-slate-400">{{ t('states.searchPageHint') }}</p>
    </div>
    <StateBlock :loading="isPending" :error="!!error && !forbidden" :detail="errorDetail" :retry="true" @retry="refetch">
      <template v-if="forbidden">
        <div role="alert" class="flex items-start gap-2 rounded-lg border border-amber-200 bg-amber-50 p-4 text-sm text-amber-800 dark:border-amber-900 dark:bg-amber-950 dark:text-amber-200">
          <AppIcon name="alert" :size="17" class="mt-0.5 shrink-0" aria-hidden="true" />
          {{ t('errors.notAdmin') }}
        </div>
      </template>
      <ul v-else class="grid grid-cols-1 gap-4 2xl:grid-cols-2">
        <li v-for="c in visibleItems" :key="c.id">
          <AppCard :label="`${c.namespace}/${c.pluginId}@${c.version}`">
            <div class="flex flex-wrap items-center gap-x-3 gap-y-2">
              <span class="break-all font-mono text-sm font-semibold">{{ c.namespace }}/{{ c.pluginId }}@{{ c.version }}</span>
              <AppBadge v-if="c.yanked" tone="danger" icon="ban">yanked</AppBadge>
              <AppBadge v-if="c.advisory" tone="warn" icon="flag">{{ c.advisory.severity }}</AppBadge>
            </div>
            <p class="mt-1 text-xs text-slate-500 tabular-nums dark:text-slate-400">
              {{ c.target }} · {{ (c.size / 1024).toFixed(1) }} KiB
            </p>
            <p class="mt-3 break-all rounded-xl bg-slate-50 p-3 font-mono text-xs leading-5 text-slate-500 dark:bg-slate-800/60 dark:text-slate-400">sha256:{{ c.sha256 }} · key {{ c.keyId }}</p>
            <div class="mt-4 flex flex-wrap gap-2 border-t border-slate-100 pt-4 dark:border-slate-800">
              <AppButton size="sm" variant="secondary" icon="eye" @click="inspect(c.id)">
                {{ t('moderation.inspect') }}
              </AppButton>
              <AppButton size="sm" variant="primary" icon="check" :disabled="c.yanked" @click="askDecide(c.id, 'approve')">
                {{ t('moderation.approve') }}
              </AppButton>
              <AppButton size="sm" variant="secondary" icon="close" :disabled="c.yanked" @click="askDecide(c.id, 'reject')">
                {{ t('moderation.reject') }}
              </AppButton>
              <AppButton size="sm" variant="danger" icon="ban" :disabled="c.yanked" @click="askYank(c.id)">
                {{ t('moderation.yank') }}
              </AppButton>
              <AppButton size="sm" variant="ghost" icon="flag" @click="openAdvisory(c.id)">
                {{ t('moderation.advisory') }}
              </AppButton>
            </div>
            <p v-if="links[c.id]" class="mt-2 break-all text-xs text-slate-600 dark:text-slate-400">
              {{ t('moderation.inspectionReady') }}
              <a :href="links[c.id]" class="font-medium text-slate-900 underline dark:text-white" target="_blank" rel="noreferrer noopener">
                {{ links[c.id] }}
                <AppIcon name="external" :size="12" class="inline" aria-hidden="true" />
              </a>
            </p>
            <form v-if="advisoryFor === c.id" class="mt-3 flex flex-col gap-3 rounded-lg border border-slate-200 p-3 dark:border-slate-700" @submit.prevent="saveAdvisory(c.id)">
              <AppSelect v-model="adv.severity" :label="t('moderation.severity')">
                <option>low</option><option>moderate</option><option>high</option><option>critical</option>
              </AppSelect>
              <AppInput v-model="adv.message" :label="t('moderation.message')" required :maxlength="1024" icon="flag" />
              <AppInput v-model="adv.url" :label="t('moderation.url')" type="url" icon="external" placeholder="https://" />
              <div class="flex flex-wrap gap-2">
                <AppButton type="submit" size="sm" variant="primary" icon="check">{{ t('states.confirm') }}</AppButton>
                <AppButton type="button" size="sm" variant="secondary" @click="advisoryFor = null">{{ t('states.cancel') }}</AppButton>
              </div>
            </form>
          </AppCard>
        </li>
      </ul>
      <EmptyState v-if="!forbidden && visibleItems.length === 0" :title="search ? t('states.noMatches') : t('states.empty')" icon="moderation" />
      <div v-if="data?.nextCursor" class="mt-3 flex justify-center">
        <AppButton variant="secondary" icon="chevronDown" @click="after = data.nextCursor ?? undefined">
          {{ t('states.next') }}
        </AppButton>
      </div>
    </StateBlock>
    <ConfirmDialog
      :danger="confirmDanger"
      :open="confirmOpen"
      :title="confirmTitle"
      :body="confirmBody"
      :busy="busy"
      :prompt-label="yankArmed ? t('moderation.yankReason') : undefined"
      v-model:prompt-value="yankReason"
      @confirm="doConfirm"
      @cancel="closeConfirm"
    />
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
import AppIcon from '../components/icons/AppIcon.vue'
import AppBadge from '../components/ui/AppBadge.vue'
import AppButton from '../components/ui/AppButton.vue'
import AppCard from '../components/ui/AppCard.vue'
import AppInput from '../components/ui/AppInput.vue'
import AppSelect from '../components/ui/AppSelect.vue'
import EmptyState from '../components/ui/EmptyState.vue'
import AppSearch from '../components/ui/AppSearch.vue'
import PageHeader from '../components/ui/PageHeader.vue'

const { t } = useI18n()
const ui = useUiStore()
const client = useQueryClient()
const after = ref<string | undefined>(undefined)
const search = ref('')

const { data, isPending, error, refetch } = useQuery({
  queryKey: computed(() => ['moderation', 'queue', after.value]),
  queryFn: () => api.moderationQueue(after.value),
})
const items = computed(() => data.value?.items ?? [])
const visibleItems = computed(() => {
  const query = search.value.trim().toLowerCase()
  return query ? items.value.filter((c) => [c.namespace, c.pluginId, c.version, c.target, c.sha256].some((text) => text.toLowerCase().includes(query))) : items.value
})
const forbidden = computed(() => error.value instanceof ApiError && error.value.status === 403)
const errorDetail = computed(() =>
  error.value instanceof ApiError && error.value.status !== 403 ? `HTTP ${error.value.status}` : undefined,
)

const links = reactive<Record<string, string>>({})
const confirmOpen = ref(false)
const confirmTitle = ref('')
const confirmBody = ref('')
const confirmDanger = ref(true)
const busy = ref(false)
const yankArmed = ref(false)
const yankReason = ref('')
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
  yankArmed.value = false
  yankReason.value = ''
  confirmDanger.value = decision === 'reject'
  confirmTitle.value = decision === 'approve' ? t('moderation.approve') : t('moderation.reject')
  confirmBody.value = t('moderation.decideConfirm')
  confirmAction = async () => {
    await api.decideRelease(id, decision, decision === 'approve' ? 'policy_reviewed' : 'policy')
    ui.toast(t('moderation.decided'), 'ok')
  }
  confirmOpen.value = true
}

function askYank(id: string) {
  yankArmed.value = true
  yankReason.value = ''
  confirmDanger.value = true
  confirmTitle.value = t('moderation.yank')
  confirmBody.value = t('moderation.yankConfirm')
  confirmAction = async () => {
    await api.yankRelease(id, yankReason.value.trim())
    ui.toast(t('moderation.yanked'), 'ok')
  }
  confirmOpen.value = true
}

function closeConfirm() {
  confirmOpen.value = false
  yankArmed.value = false
  yankReason.value = ''
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
    closeConfirm()
    await client.invalidateQueries({ queryKey: ['moderation'] })
    await client.invalidateQueries({ queryKey: ['admin'] })
    await refetch()
  } catch (e) {
    ui.toast(t('errors.actionFailed'), 'err')
    if (e instanceof ApiError && (e.status === 403 || e.status === 409)) closeConfirm()
  } finally {
    busy.value = false
  }
}
</script>
