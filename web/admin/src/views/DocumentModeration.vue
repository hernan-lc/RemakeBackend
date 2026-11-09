<template>
  <div class="flex flex-col gap-4">
    <PageHeader :title="copy.title" :description="copy.description" />
    <RouterLink to="/app/moderation" class="self-start text-sm font-medium text-indigo-700 underline dark:text-indigo-300">{{ copy.releases }}</RouterLink>
    <div class="flex flex-wrap gap-2">
      <AppButton :variant="mode === 'pending' ? 'primary' : 'secondary'" @click="changeMode('pending')">{{ copy.pending }}</AppButton>
      <AppButton :variant="mode === 'published' ? 'primary' : 'secondary'" @click="changeMode('published')">{{ copy.published }}</AppButton>
      <AppButton variant="secondary" :disabled="busy" @click="reload">{{ copy.refresh }}</AppButton>
    </div>
    <p class="text-sm text-slate-700 dark:text-slate-300">{{ copy.reviewHint }}</p>
    <StateBlock :loading="isPending" :error="!!error" :detail="errorDetail" :retry="true" @retry="reload">
      <div v-if="items.length" class="grid min-w-0 items-start gap-5 xl:grid-cols-[minmax(16rem,0.8fr)_minmax(0,1.7fr)]">
        <ul class="max-h-96 overflow-y-auto divide-y divide-slate-200 rounded-lg border border-slate-200 dark:divide-slate-700 dark:border-slate-700">
          <li v-for="item in items" :key="`${item.id}:${item.revision}`">
            <button type="button" class="w-full border-l-4 p-4 text-left" :class="selected?.id === item.id ? 'border-indigo-600 bg-indigo-50 dark:bg-slate-800' : 'border-transparent'" :aria-pressed="selected?.id === item.id" @click="select(item)">
              <span class="block break-words font-semibold">{{ item.title || item.slug }}</span>
              <span class="text-xs">{{ item.kind }} · {{ copy.revision }} {{ item.revision }}</span>
            </button>
          </li>
        </ul>
        <section v-if="selected" class="min-w-0 rounded-lg border border-slate-200 bg-white p-4 dark:border-slate-700 dark:bg-slate-900">
          <h2 class="break-words text-lg font-semibold">{{ selected.title || selected.slug }}</h2>
          <dl class="mt-3 space-y-2 text-xs">
            <div><dt class="font-semibold">ID</dt><dd class="break-all font-mono">{{ selected.id }}</dd></div>
            <div><dt class="font-semibold">{{ copy.revision }}</dt><dd>{{ selected.revision }}</dd></div>
            <div><dt class="font-semibold">SHA-256</dt><dd class="break-all font-mono">{{ selected.sha256 }}</dd></div>
          </dl>
          <p v-if="detailPending" class="mt-4">{{ copy.loading }}</p>
          <p v-else-if="detailError" role="alert" class="mt-4 text-red-700 dark:text-red-300">{{ copy.detailError }}</p>
          <template v-else-if="reviewedDocument">
            <h3 class="mt-4 text-sm font-semibold">{{ copy.content }}</h3>
            <pre data-testid="document-content" class="mt-2 max-h-96 overflow-auto whitespace-pre-wrap break-all rounded bg-slate-100 p-3 text-xs dark:bg-slate-800">{{ JSON.stringify(reviewedDocument.document, null, 2) }}</pre>
            <div class="mt-4 flex flex-wrap gap-2">
              <template v-if="mode === 'pending'">
                <AppButton :disabled="busy" @click="prepare('approve')">{{ copy.approve }}</AppButton>
                <AppButton variant="secondary" :disabled="busy" @click="prepare('reject')">{{ copy.reject }}</AppButton>
              </template>
              <AppButton v-else variant="danger" :disabled="busy" @click="prepare('withdraw')">{{ copy.withdraw }}</AppButton>
            </div>
          </template>
        </section>
      </div>
      <p v-else class="rounded-lg border border-slate-200 p-4 text-sm dark:border-slate-700">{{ copy.empty }}</p>
      <div class="mt-4 flex gap-2">
        <AppButton v-if="after" variant="secondary" :disabled="busy" @click="firstPage">{{ copy.first }}</AppButton>
        <AppButton v-if="data?.nextCursor" variant="secondary" :disabled="busy" @click="nextPage">{{ copy.next }}</AppButton>
      </div>
    </StateBlock>
    <ConfirmDialog :open="!!action && !!target" :title="actionLabel" :body="confirmBody" :prompt-label="`${copy.reason} (${reasonBytes}/500 ${copy.bytes})`" v-model:prompt-value="reason" :busy="busy" :confirm-disabled="!validReason" :danger="action !== 'approve'" @confirm="submit" @cancel="cancelAction" />
    <p v-if="resultMessage" role="status" class="text-sm">{{ resultMessage }}</p>
  </div>
</template>

<script setup lang="ts">
import { computed, ref, watch } from 'vue'
import { useI18n } from 'vue-i18n'
import { useQuery, useQueryClient } from '@tanstack/vue-query'
import { ApiError, api } from '../api/client'
import type { CloudDocument, CloudDocumentSummary } from '../api/types'
import { useSession } from '../queries/session'
import StateBlock from '../components/StateBlock.vue'
import ConfirmDialog from '../components/ConfirmDialog.vue'
import AppButton from '../components/ui/AppButton.vue'
import PageHeader from '../components/ui/PageHeader.vue'

const { locale } = useI18n()
const copy = computed(() => locale.value === 'es' ? {
  title: 'Moderación de documentos Cloud', description: 'Revisa plantillas y perfiles publicados o pendientes de otros autores.', releases: 'Volver a moderación de releases', pending: 'Pendientes', published: 'Publicados', refresh: 'Actualizar', reviewHint: 'Revisa todo el contenido y sus acciones. La aprobación de Cloud no autoriza su ejecución en la aplicación.', revision: 'Revisión', loading: 'Cargando contenido…', detailError: 'No se pudo verificar esta revisión; actualiza la lista.', content: 'Contenido completo', approve: 'Aprobar', reject: 'Rechazar', withdraw: 'Retirar publicación', empty: 'No hay documentos en esta página.', first: 'Primera página', next: 'Página siguiente', confirm: 'Confirmar', reason: 'Motivo obligatorio', bytes: 'bytes', cancel: 'Cancelar', success: 'Decisión guardada. Se actualizó la lista.', uncertain: 'No se pudo confirmar el resultado. Actualiza y revisa el estado antes de volver a intentar.', forbidden: 'Tu sesión no tiene permiso para moderar estos documentos.', failed: 'No se pudo cargar la lista.',
} : {
  title: 'Cloud document moderation', description: 'Review pending and published templates and profiles.', releases: 'Back to release moderation', pending: 'Pending', published: 'Published', refresh: 'Refresh', reviewHint: 'Review all content and actions. Cloud approval does not authorize execution in the application.', revision: 'Revision', loading: 'Loading content…', detailError: 'This revision could not be verified; refresh the list.', content: 'Full content', approve: 'Approve', reject: 'Reject', withdraw: 'Withdraw publication', empty: 'There are no documents on this page.', first: 'First page', next: 'Next page', confirm: 'Confirm', reason: 'Required reason', bytes: 'bytes', cancel: 'Cancel', success: 'Decision saved. The list has been refreshed.', uncertain: 'The result could not be confirmed. Refresh and review the status before trying again.', forbidden: 'Your session cannot moderate these documents.', failed: 'The list could not be loaded.',
})

const mode = ref<'pending' | 'published'>('pending')
const after = ref<string | undefined>()
const selectedId = ref<string>()
const action = ref<'approve' | 'reject' | 'withdraw' | null>(null)
const target = ref<CloudDocumentSummary | null>(null)
const reason = ref('')
const busy = ref(false)
const resultMessage = ref('')
const session = useSession()
const queryClient = useQueryClient()
const userId = computed(() => session.error.value ? undefined : session.data.value?.id)
async function sessionBound<T>(actor: string, operation: () => Promise<T>): Promise<T> {
  try {
    if ((await api.me()).id !== actor || userId.value !== actor) throw new Error('session changed')
    const result = await operation()
    if ((await api.me()).id !== actor || userId.value !== actor) throw new Error('session changed')
    return result
  } catch (error) {
    void session.refetch()
    throw error
  }
}
const { data, isPending, error, refetch } = useQuery({
  queryKey: computed(() => ['cloud-document-review', userId.value, mode.value, after.value]),
  enabled: computed(() => !!userId.value),
  queryFn: ({ queryKey }) => sessionBound(queryKey[1] as string, () => queryKey[2] === 'pending' ? api.documentReviewQueue(queryKey[3] as string | undefined) : api.publishedDocuments(queryKey[3] as string | undefined)),
  retry: false,
})
const items = computed(() => error.value || !userId.value ? [] : data.value?.items ?? [])
const selected = computed(() => items.value.find((item) => item.id === selectedId.value) ?? items.value[0])
const detailQuery = useQuery({
  queryKey: computed(() => ['cloud-document-review-content', userId.value, selected.value?.id, selected.value?.revision, selected.value?.sha256]),
  enabled: computed(() => mode.value === 'published' && !!userId.value && !!selected.value && Number.isSafeInteger(selected.value.revision) && selected.value.revision > 0),
  queryFn: ({ queryKey }) => sessionBound(queryKey[1] as string, () => api.publishedDocumentVersion(queryKey[2] as string, queryKey[3] as number)),
  retry: false,
})
const reviewedDocument = computed<CloudDocument | null>(() => {
  const item = selected.value
  if (!item || !Number.isSafeInteger(item.revision) || item.revision < 1) return null
  const doc = mode.value === 'pending' ? item as CloudDocument : detailQuery.data.value
  if (!doc || !('document' in doc) || doc.document == null || doc.id !== item.id || doc.revision !== item.revision || doc.sha256 !== item.sha256 || doc.status !== mode.value) return null
  return doc
})
const detailPending = computed(() => mode.value === 'published' && detailQuery.isPending.value && detailQuery.fetchStatus.value === 'fetching')
const detailError = computed(() => !detailPending.value && !reviewedDocument.value)
const errorDetail = computed(() => error.value instanceof ApiError && error.value.status === 403 ? copy.value.forbidden : copy.value.failed)
const reasonBytes = computed(() => new TextEncoder().encode(reason.value.trim()).length)
const validReason = computed(() => reasonBytes.value > 0 && reasonBytes.value <= 500)
const actionLabel = computed(() => action.value === 'approve' ? copy.value.approve : action.value === 'reject' ? copy.value.reject : copy.value.withdraw)
const confirmBody = computed(() => target.value ? `${target.value.title || target.value.slug} · ${target.value.id} · ${copy.value.revision} ${target.value.revision} · SHA-256 ${target.value.sha256}` : '')

function select(item: CloudDocumentSummary) { selectedId.value = item.id; action.value = null; resultMessage.value = '' }
function changeMode(next: 'pending' | 'published') { if (busy.value) return; mode.value = next; after.value = undefined; selectedId.value = undefined; action.value = null; resultMessage.value = '' }
function firstPage() { after.value = undefined; selectedId.value = undefined; action.value = null }
function cancelAction() { if (!busy.value) action.value = null }
function nextPage() { if (data.value?.nextCursor) { after.value = data.value.nextCursor; selectedId.value = undefined; action.value = null } }
async function reload() { action.value = null; selectedId.value = undefined; if (after.value) firstPage(); else await refetch() }
function prepare(next: 'approve' | 'reject' | 'withdraw') {
  if (!reviewedDocument.value || busy.value) return
  target.value = { ...reviewedDocument.value }
  action.value = next
  reason.value = ''
  resultMessage.value = ''
}
async function submit() {
  const frozen = target.value
  const operation = action.value
  const reviewed = reviewedDocument.value
  const actor = userId.value
  if (!frozen || !operation || !reviewed || !actor || !validReason.value || busy.value || frozen.id !== reviewed.id || frozen.revision !== reviewed.revision || frozen.sha256 !== reviewed.sha256) return
  busy.value = true
  try {
    const current = await api.me()
    if (current.id !== actor || userId.value !== actor || action.value !== operation || target.value !== frozen) throw new Error('session or review changed')
    if (operation === 'withdraw') await api.withdrawDocument(frozen.id, frozen.revision, reason.value.trim())
    else await api.decideDocument(frozen.id, frozen.revision, operation, reason.value.trim())
    resultMessage.value = copy.value.success
  } catch {
    resultMessage.value = copy.value.uncertain
  } finally {
    action.value = null
    target.value = null
    selectedId.value = undefined
    after.value = undefined
    await queryClient.invalidateQueries({ queryKey: ['cloud-document-review'] })
    busy.value = false
  }
}
watch(userId, (next, previous) => { if (next !== previous) { action.value = null; target.value = null; selectedId.value = undefined; after.value = undefined; resultMessage.value = '' } })
</script>
