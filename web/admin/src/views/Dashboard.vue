<template>
  <div class="flex flex-col gap-6">
    <PageHeader :title="t('app.dashboard')" :description="copy.description">
      <template #actions>
        <AppButton size="sm" variant="secondary" icon="refresh" @click="refetchAll">{{ t('states.refresh') }}</AppButton>
        <AppButton v-if="isAdmin" variant="primary" icon="arrowRight" to="/app/moderation">{{ copy.openQueue }}</AppButton>
      </template>
    </PageHeader>
    <AppNotice v-if="!isAdmin" tone="warn">{{ t('errors.notAdmin') }}</AppNotice>
    <template v-else>
      <StateBlock :loading="ovPending" :error="!!ovError" :retry="true" @retry="refetchOv">
        <div class="grid grid-cols-1 gap-3 sm:grid-cols-3">
          <StatCard :label="t('dashboard.pendingPublishers')" :value="overview?.publishers.pending ?? '—'" icon="publishers" tone="indigo" to="/app/publishers" />
          <StatCard :label="t('dashboard.pendingKeys')" :value="overview?.pendingKeys ?? '—'" icon="key" tone="emerald" to="/app/publishers" />
          <StatCard :label="copy.loaded" :value="queuePending || queueError ? '—' : queue?.items.length ?? '—'" :hint="copy.pageHint" icon="moderation" tone="amber" to="/app/moderation" />
        </div>
        <p v-if="dataUpdatedAt" class="mt-2 text-xs text-slate-600 dark:text-slate-300">{{ copy.updated }} · {{ formatDateTime(new Date(dataUpdatedAt).toISOString(), locale) }}</p>
      </StateBlock>

      <div class="grid items-start gap-5 xl:grid-cols-[minmax(0,1.65fr)_minmax(0,1fr)]">
        <AppCard label="upcoming-reviews">
          <AppSectionHeader :title="copy.upcoming">
            <template #actions><AppBadge tone="neutral">{{ queuePending || queueError ? '—' : queue?.items.length ?? '—' }}</AppBadge></template>
          </AppSectionHeader>
          <p class="mt-2 text-sm leading-6 text-slate-600 dark:text-slate-300">{{ copy.independent }}</p>
          <div class="mt-4">
            <StateBlock :loading="queuePending" :error="!!queueError" :retry="true" @retry="refetchQueue">
              <ul v-if="queue?.items.length" class="divide-y divide-slate-200 dark:divide-slate-700">
                <li v-for="release in queue.items.slice(0, 5)" :key="release.id" class="flex flex-wrap items-center gap-3 py-4 first:pt-0 last:pb-0">
                  <div class="min-w-0 flex-1 basis-48">
                    <p class="break-all text-sm font-semibold text-slate-950 dark:text-white">{{ release.namespace }} / {{ release.pluginId }}</p>
                    <p class="mt-1 break-words text-xs text-slate-600 dark:text-slate-300">{{ release.version }} · {{ release.target }}</p>
                  </div>
                  <AppBadge tone="warn">{{ copy.pending }}</AppBadge>
                  <AppButton size="sm" variant="secondary" to="/app/moderation" :aria-label="`${copy.review}: ${release.namespace}/${release.pluginId}`">{{ copy.review }}</AppButton>
                </li>
              </ul>
              <EmptyState v-else :title="copy.emptyQueue" icon="moderation" />
              <p v-if="queue?.nextCursor || (queue?.items.length ?? 0) > 5" class="mt-4 border-t border-slate-200 pt-3 text-sm text-slate-600 dark:border-slate-700 dark:text-slate-300">{{ copy.more }}</p>
            </StateBlock>
          </div>
        </AppCard>

        <AppCard label="recent-audit">
          <AppSectionHeader :title="copy.activity">
            <template #actions><AppButton size="sm" variant="ghost" icon="arrowRight" to="/app/audit">{{ t('app.audit') }}</AppButton></template>
          </AppSectionHeader>
          <p v-if="overview && !ovError" class="mt-2 text-xs text-slate-600 dark:text-slate-300">{{ copy.events }}: <strong class="tabular-nums">{{ overview.auditEvents }}</strong></p>
          <div class="mt-4">
            <StateBlock :loading="auditPending" :error="!!auditError" :retry="true" @retry="refetchAudit">
              <ul v-if="audit?.events.length" class="divide-y divide-slate-200 dark:divide-slate-700">
                <li v-for="event in audit.events.slice(0, 5)" :key="event.id" class="py-3 first:pt-0 last:pb-0">
                  <p class="break-words text-sm font-medium text-slate-950 dark:text-white">{{ auditLabels[event.action] ?? event.action }}</p>
                  <p class="mt-1 break-words text-xs text-slate-600 dark:text-slate-300">{{ formatDateTime(event.createdAt, locale) }} · <span :title="event.actorUserId">{{ compactId(event.actorUserId) }}</span></p>
                  <code v-if="auditLabels[event.action]" class="mt-1 block break-all text-[11px] text-slate-600 dark:text-slate-400">{{ event.action }}</code>
                </li>
              </ul>
              <EmptyState v-else :title="t('dashboard.noActivity')" icon="audit" />
            </StateBlock>
          </div>
        </AppCard>
      </div>

      <StateBlock v-if="!ovError" :loading="ovPending" :error="false">
        <AppCard label="releases-status">
          <AppSectionHeader :title="t('dashboard.releases')" />
          <div v-if="releaseRows.length" class="mt-4 grid gap-x-8 gap-y-4 sm:grid-cols-2 xl:grid-cols-3">
            <AppStatusBar v-for="row in releaseRows" :key="row.key" :label="row.label" :value="row.value" :total="totalReleases" :tone="row.tone" />
          </div>
          <EmptyState v-else :title="t('states.empty')" icon="package" />
        </AppCard>
      </StateBlock>
    </template>
  </div>
</template>

<script setup lang="ts">
import { computed } from 'vue'
import { useI18n } from 'vue-i18n'
import { useQuery } from '@tanstack/vue-query'
import AppBadge from '../components/ui/AppBadge.vue'
import { api } from '../api/client'
import { isCloudAdmin } from '../api/types'
import { useSession } from '../queries/session'
import { compactId, formatDateTime } from '../utils/format'
import StateBlock from '../components/StateBlock.vue'
import AppNotice from '../components/ui/AppNotice.vue'
import AppButton from '../components/ui/AppButton.vue'
import AppCard from '../components/ui/AppCard.vue'
import AppSectionHeader from '../components/ui/AppSectionHeader.vue'
import AppStatusBar from '../components/ui/AppStatusBar.vue'
import EmptyState from '../components/ui/EmptyState.vue'
import PageHeader from '../components/ui/PageHeader.vue'
import StatCard from '../components/ui/StatCard.vue'

const { t, locale } = useI18n()
const copy = computed(() => locale.value === 'es' ? {
  description: 'Revisa solicitudes, claves y releases desde un único lugar.', openQueue: 'Abrir cola de revisión',
  loaded: 'Releases cargadas', pageHint: 'Recuento de esta página, no del total de la cola.', more: 'Hay más resultados en la cola.',
  upcoming: 'Próximas revisiones', review: 'Revisar', emptyQueue: 'No hay releases en esta página',
  independent: 'Las decisiones requieren revisión independiente en la cola.', updated: 'Resumen actualizado',
  activity: 'Actividad reciente', events: 'Eventos registrados', unknown: 'Sin datos', pending: 'Pendiente de revisión',
} : {
  description: 'Review publisher requests, signing keys and releases in one place.', openQueue: 'Open review queue',
  loaded: 'Releases loaded', pageHint: 'Count for this page, not the entire queue.', more: 'More results are available in the queue.',
  upcoming: 'Upcoming reviews', review: 'Review', emptyQueue: 'No releases on this page',
  independent: 'Decisions require independent review in the moderation queue.', updated: 'Overview updated',
  activity: 'Recent activity', events: 'Recorded events', unknown: 'No data', pending: 'Awaiting review',
})
const auditLabels = computed<Record<string, string>>(() => locale.value === 'es' ? {
  'publisher.created': 'Publicador solicitado', 'publisher.approved': 'Publicador aprobado',
  'release.submitted': 'Release enviada', 'key.approved': 'Clave aprobada',
} : {
  'publisher.created': 'Publisher requested', 'publisher.approved': 'Publisher approved',
  'release.submitted': 'Release submitted', 'key.approved': 'Signing key approved',
})
const { data: me } = useSession()
const isAdmin = computed(() => isCloudAdmin(me.value))

const { data: overview, isPending: ovPending, error: ovError, dataUpdatedAt, refetch: refetchOv } = useQuery({
  queryKey: ['admin', 'overview'],
  queryFn: api.overview,
  enabled: isAdmin,
})
const { data: audit, isPending: auditPending, error: auditError, refetch: refetchAudit } = useQuery({
  queryKey: ['admin', 'audit', 'recent'],
  queryFn: () => api.audit({ limit: 8 }),
  enabled: isAdmin,
})
const { data: queue, isPending: queuePending, error: queueError, refetch: refetchQueue } = useQuery({
  queryKey: ['moderation', 'queue', 'count'],
  queryFn: () => api.moderationQueue(),
  enabled: isAdmin,
})

const totalReleases = computed(() => overview.value ? Object.values(overview.value.releases).reduce((sum, count) => sum + count, 0) : 0)
const releaseRows = computed(() => overview.value ? [
  { key: 'awaitingVerification', label: t('dashboard.awaitingVerification'), value: overview.value.releases.awaitingVerification, tone: 'info' as const },
  { key: 'integrityPassed', label: t('dashboard.integrityPassed'), value: overview.value.releases.integrityPassed, tone: 'info' as const },
  { key: 'antivirusPassed', label: t('dashboard.antivirusPassed'), value: overview.value.releases.antivirusPassed, tone: 'success' as const },
  { key: 'reviewApproved', label: t('dashboard.reviewApproved'), value: overview.value.releases.reviewApproved, tone: 'success' as const },
  { key: 'reviewRejected', label: t('dashboard.reviewRejected'), value: overview.value.releases.reviewRejected, tone: 'warn' as const },
  { key: 'rejected', label: t('dashboard.rejected'), value: overview.value.releases.rejected, tone: 'danger' as const },
].filter((row) => row.value > 0) : [])

function refetchAll() {
  void refetchOv()
  void refetchAudit()
  void refetchQueue()
}
</script>
