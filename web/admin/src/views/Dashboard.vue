<template>
  <div class="flex flex-col gap-6 sm:gap-7">
    <PageHeader :title="t('dashboard.title')" :description="t('dashboard.description')" icon="dashboard">
      <template #actions>
        <AppButton size="sm" variant="secondary" icon="refresh" @click="refetchAll">{{ t('states.refresh') }}</AppButton>
      </template>
    </PageHeader>

    <StateBlock :loading="isAdmin && (ovPending || auditPending || queuePending)" :error="isAdmin && !!ovError" :retry="true" @retry="refetchAll">
      <AppNotice v-if="!isAdmin" tone="warn">{{ t('errors.notAdmin') }}</AppNotice>
      <template v-else>
        <dl class="grid grid-cols-1 gap-3 sm:grid-cols-2 xl:grid-cols-4">
          <StatCard :label="t('dashboard.pendingPublishers')" :value="overview?.publishers.pending ?? 0" :hint="t('dashboard.publisherHint')" icon="publishers" tone="indigo" to="/app/publishers" />
          <StatCard :label="t('dashboard.reviewQueue')" :value="queue?.items.length ?? 0" :hint="t('dashboard.queueHint')" icon="moderation" tone="amber" to="/app/moderation" />
          <StatCard :label="t('dashboard.pendingKeys')" :value="overview?.pendingKeys ?? 0" :hint="t('dashboard.keysHint')" icon="key" tone="emerald" />
          <StatCard :label="t('dashboard.auditEvents')" :value="overview?.auditEvents ?? 0" :hint="t('dashboard.auditHint')" icon="audit" tone="sky" to="/app/audit" />
        </dl>

        <div class="mt-6 grid grid-cols-1 gap-4 xl:grid-cols-[minmax(0,1.1fr)_minmax(0,0.9fr)]">
          <AppCard label="releases-status">
            <AppSectionHeader :title="t('dashboard.releases')" :description="t('dashboard.releasesHint')" />
            <div v-if="overview" class="mt-6 grid gap-x-7 gap-y-5 sm:grid-cols-2 xl:grid-cols-1 2xl:grid-cols-2">
              <AppStatusBar v-for="row in releaseRows" :key="row.key" :label="row.label" :value="row.value" :total="totalReleases" :tone="row.tone" />
            </div>
            <EmptyState v-else :title="t('states.empty')" icon="package" />
          </AppCard>

          <AppCard label="recent-audit">
            <AppSectionHeader :title="t('dashboard.recentAudit')" :description="t('dashboard.activityHint')">
              <template #actions><AppButton size="sm" variant="ghost" icon="arrowRight" to="/app/audit">{{ t('app.audit') }}</AppButton></template>
            </AppSectionHeader>
            <ul v-if="(audit?.events.length ?? 0) > 0" class="mt-5 divide-y divide-slate-100 dark:divide-slate-800">
              <li v-for="e in audit?.events.slice(0, 8) ?? []" :key="e.id" class="flex gap-3 py-3 first:pt-0">
                <span class="mt-1 flex size-8 shrink-0 items-center justify-center rounded-xl bg-slate-100 text-slate-500 dark:bg-slate-800 dark:text-slate-300"><AppIcon name="audit" :size="15" aria-hidden="true" /></span>
                <div class="min-w-0 flex-1">
                  <p class="break-all font-mono text-xs font-semibold text-slate-800 sm:text-sm dark:text-slate-200">{{ e.action }}</p>
                  <p class="mt-1 text-xs text-slate-500 dark:text-slate-400">{{ formatDateTime(e.createdAt, locale) }} · {{ compactId(e.actorUserId) }}</p>
                </div>
              </li>
            </ul>
            <EmptyState v-else :title="t('dashboard.noActivity')" icon="audit" />
          </AppCard>
        </div>
      </template>
    </StateBlock>
  </div>
</template>

<script setup lang="ts">
import { computed } from 'vue'
import { useI18n } from 'vue-i18n'
import { useQuery } from '@tanstack/vue-query'
import { api } from '../api/client'
import { isCloudAdmin } from '../api/types'
import { useSession } from '../queries/session'
import { compactId, formatDateTime } from '../utils/format'
import StateBlock from '../components/StateBlock.vue'
import AppIcon from '../components/icons/AppIcon.vue'
import AppNotice from '../components/ui/AppNotice.vue'
import AppButton from '../components/ui/AppButton.vue'
import AppCard from '../components/ui/AppCard.vue'
import AppSectionHeader from '../components/ui/AppSectionHeader.vue'
import AppStatusBar from '../components/ui/AppStatusBar.vue'
import EmptyState from '../components/ui/EmptyState.vue'
import PageHeader from '../components/ui/PageHeader.vue'
import StatCard from '../components/ui/StatCard.vue'

const { t, locale } = useI18n()
const { data: me } = useSession()
const isAdmin = computed(() => isCloudAdmin(me.value))

const { data: overview, isPending: ovPending, error: ovError, refetch: refetchOv } = useQuery({
  queryKey: ['admin', 'overview'],
  queryFn: api.overview,
  enabled: isAdmin,
})
const { data: audit, isPending: auditPending, refetch: refetchAudit } = useQuery({
  queryKey: ['admin', 'audit', 'recent'],
  queryFn: () => api.audit({ limit: 8 }),
  enabled: isAdmin,
})
const { data: queue, isPending: queuePending, refetch: refetchQueue } = useQuery({
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
] : [])

function refetchAll() {
  void refetchOv()
  void refetchAudit()
  void refetchQueue()
}
</script>
