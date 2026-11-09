<template>
  <div class="flex flex-col gap-4">
    <PageHeader :title="t('moderation.title')" :description="copy.description" />
    <RouterLink to="/app/moderation/documents" class="self-start rounded-lg px-3 py-2 text-sm font-medium text-indigo-700 underline dark:text-indigo-300">{{ locale === 'es' ? 'Moderar documentos Cloud' : 'Review Cloud documents' }}</RouterLink>
    <AppToolbar v-model="search" :search-label="t('moderation.searchPage')" :refresh-label="t('states.refresh')" @refresh="refetch" />
    <StateBlock :loading="isPending" :error="!!error && !forbidden" :detail="errorDetail" :retry="true" @retry="refetch">
      <template v-if="forbidden">
        <div role="alert" class="flex items-start gap-2 rounded-lg border border-amber-200 bg-amber-50 p-4 text-sm text-amber-800 dark:border-amber-900 dark:bg-amber-950 dark:text-amber-200">
          <AppIcon name="alert" :size="17" class="mt-0.5 shrink-0" aria-hidden="true" />
          {{ t('errors.notAdmin') }}
        </div>
      </template>
      <div v-else-if="visibleItems.length" class="grid min-w-0 items-start gap-5 xl:grid-cols-[minmax(16rem,0.8fr)_minmax(0,1.7fr)]">
        <section class="min-w-0 rounded-xl border border-slate-200 bg-white dark:border-slate-700 dark:bg-slate-900" :aria-label="copy.queue">
          <div class="flex flex-wrap items-center justify-between gap-2 border-b border-slate-200 p-4 dark:border-slate-700">
            <h2 class="text-sm font-semibold">{{ copy.queue }}</h2>
            <span class="text-xs text-slate-600 dark:text-slate-300">{{ visibleItems.length }} {{ copy.loaded }}</span>
          </div>
          <ul class="max-h-72 overflow-y-auto divide-y divide-slate-200 xl:max-h-[calc(100vh-18rem)] dark:divide-slate-700">
            <li v-for="item in visibleItems" :key="item.id">
              <button type="button" class="flex min-h-20 w-full flex-col gap-1 border-l-4 p-4 text-left transition-colors focus-visible:outline-2 focus-visible:outline-offset-[-2px] focus-visible:outline-indigo-600"
                :class="selectedRelease?.id === item.id ? 'border-indigo-600 bg-indigo-50 text-slate-950 dark:border-indigo-400 dark:bg-slate-800 dark:text-white' : 'border-transparent text-slate-700 hover:bg-slate-50 dark:text-slate-200 dark:hover:bg-slate-800'"
                aria-controls="release-review-detail" :aria-pressed="selectedRelease?.id === item.id" :aria-label="`${copy.select}: ${item.namespace}/${item.pluginId}@${item.version}`" @click="selectedId = item.id">
                <span class="break-all text-xs text-slate-600 dark:text-slate-300">{{ item.namespace }}</span>
                <span class="break-all text-sm font-semibold">{{ item.pluginId }}</span>
                <span class="text-xs">{{ item.version }} · {{ item.target }}</span>
                <span v-if="item.yanked" class="text-xs font-semibold text-red-700 dark:text-red-300">yanked</span>
              </button>
            </li>
          </ul>
        </section>
        <section v-for="c in selectedItems" :key="c.id" id="release-review-detail" class="min-w-0" :aria-label="copy.detail">
          <AppCard :label="`${c.namespace}/${c.pluginId}@${c.version}`">
            <p class="mb-2 text-xs font-semibold uppercase tracking-wider text-slate-600 dark:text-slate-300">{{ copy.detail }}</p>
            <div class="flex flex-wrap items-center gap-x-3 gap-y-2">
              <h2 class="break-all text-xl font-semibold tracking-tight">{{ c.namespace }}/{{ c.pluginId }}<span class="ml-2 font-mono text-sm text-slate-600 dark:text-slate-300">{{ c.version }}</span></h2>
              <AppBadge v-if="c.yanked" tone="danger" icon="ban">yanked</AppBadge>
              <AppBadge v-if="c.advisory" tone="warn" icon="flag">{{ c.advisory.severity }}</AppBadge>
            </div>
            <p class="mt-1 text-xs text-slate-600 tabular-nums dark:text-slate-300">
              {{ c.target }} · {{ (c.size / 1024).toFixed(1) }} KiB
            </p>
            <p class="mt-4 border-l-2 border-indigo-600 pl-3 text-sm leading-6 text-slate-700 dark:border-indigo-400 dark:text-slate-200">{{ t('moderation.conflictNote') }}</p>
            <details class="mt-5 rounded-lg border border-slate-200 p-4 text-xs text-slate-700 dark:border-slate-700 dark:text-slate-200">
              <summary class="min-h-11 cursor-pointer py-3 font-medium">{{ t('moderation.details') }}</summary>
              <dl class="mt-3 grid gap-3 leading-5">
                <div><dt class="font-semibold">SHA-256</dt><dd class="mt-1 break-all font-mono">{{ c.sha256 }}</dd></div>
                <div><dt class="font-semibold">{{ c.keyId }}</dt><dd class="mt-1 break-all font-mono">{{ c.keyFingerprint }}</dd><dd class="text-slate-600 dark:text-slate-300">{{ copy.fingerprint }}</dd></div>
              </dl>
            </details>
            <h3 class="mt-6 text-sm font-semibold">{{ copy.decision }}</h3>
            <div class="mt-3 flex flex-wrap gap-2">
              <AppButton size="sm" variant="secondary" icon="eye" @click="inspect(c.id)">
                {{ t('moderation.inspect') }}
              </AppButton>
              <AppButton size="sm" variant="primary" icon="check" :disabled="c.yanked" @click="askDecide(c.id, 'approve')">
                {{ t('moderation.approve') }}
              </AppButton>
              <AppButton size="sm" variant="secondary" icon="close" :disabled="c.yanked" @click="askDecide(c.id, 'reject')">
                {{ t('moderation.reject') }}
              </AppButton>
              <details class="w-full sm:w-auto">
                <summary class="min-h-11 cursor-pointer rounded-lg px-3 py-3 text-xs font-medium text-slate-600 hover:bg-slate-100 dark:text-slate-300 dark:hover:bg-slate-800">{{ t('moderation.moreActions') }}</summary>
                <div class="mt-2 flex flex-wrap gap-2">
                  <AppButton size="sm" variant="danger" icon="ban" :disabled="c.yanked" @click="askYank(c.id)">{{ t('moderation.yank') }}</AppButton>
                  <AppButton size="sm" variant="ghost" icon="flag" @click="openAdvisory(c.id)">{{ t('moderation.advisory') }}</AppButton>
                </div>
              </details>
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
        </section>
      </div>
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
import AppToolbar from '../components/ui/AppToolbar.vue'
import PageHeader from '../components/ui/PageHeader.vue'

const { t, locale } = useI18n()
const copy = computed(() => locale.value === 'es' ? { queue: 'Releases en esta página', detail: 'Revisión de release', select: 'Seleccionar release', evidence: 'Evidencia del paquete', decision: 'Decisión de revisión', fingerprint: 'Huella de la clave (SHA-256)', loaded: 'cargadas', description: 'Selecciona una release, comprueba su evidencia y registra una decisión.' } : { queue: 'Releases on this page', detail: 'Release review', select: 'Select release', evidence: 'Package evidence', decision: 'Review decision', fingerprint: 'Key fingerprint (SHA-256)', loaded: 'loaded', description: 'Select a release, inspect its evidence and record a decision.' })
const selectedId = ref<string | null>(null)
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
const selectedRelease = computed(() => visibleItems.value.find(c => c.id === selectedId.value) ?? visibleItems.value[0])
const selectedItems = computed(() => selectedRelease.value ? [selectedRelease.value] : [])
const releaseLabel = (id: string) => { const item = items.value.find(c => c.id === id); return item ? `${item.namespace}/${item.pluginId}@${item.version}` : id }
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
  confirmBody.value = `${releaseLabel(id)} — ${t('moderation.decideConfirm')}`
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
  confirmBody.value = `${releaseLabel(id)} — ${t('moderation.yankConfirm')}`
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
