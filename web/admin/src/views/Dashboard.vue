<template>
  <div class="flex flex-col gap-4 sm:gap-6">
    <PageHeader :title="t('dashboard.title')" icon="dashboard">
      <template #actions>
        <AppButton size="sm" variant="secondary" icon="refresh" @click="refetchAll">
          {{ t('states.refresh') }}
        </AppButton>
      </template>
    </PageHeader>
    <StateBlock :loading="isAdmin && (ovPending || auditPending || queuePending)" :error="isAdmin && !!ovError" :retry="true" @retry="refetchAll">
      <div v-if="!isAdmin" role="alert" class="flex items-start gap-2 rounded-lg border border-amber-200 bg-amber-50 p-4 text-sm text-amber-800 dark:border-amber-900 dark:bg-amber-950 dark:text-amber-200">
        <AppIcon name="alert" :size="17" class="mt-0.5 shrink-0" aria-hidden="true" />
        {{ t('errors.notAdmin') }}
      </div>
      <template v-else>
        <dl class="grid grid-cols-1 gap-3 sm:grid-cols-2 xl:grid-cols-4">
          <StatCard :label="t('dashboard.pendingPublishers')" :value="overview?.publishers.pending ?? 0" icon="publishers" to="/app/publishers" />
          <StatCard :label="t('dashboard.reviewQueue')" :value="queue?.items.length ?? 0" icon="moderation" to="/app/moderation" />
          <StatCard :label="t('dashboard.pendingKeys')" :value="overview?.pendingKeys ?? 0" icon="key" />
          <StatCard :label="t('dashboard.auditEvents')" :value="overview?.auditEvents ?? 0" icon="audit" to="/app/audit" />
        </dl>
        <div class="mt-4 grid grid-cols-1 gap-3 sm:gap-4 xl:grid-cols-2">
          <AppCard :title="t('dashboard.releases')" label="releases">
            <ul v-if="overview" class="grid grid-cols-1 gap-1 text-sm tabular-nums text-slate-700 sm:grid-cols-2 dark:text-slate-300">
              <li class="flex items-center justify-between gap-2 rounded-md bg-slate-50 px-3 py-1.5 dark:bg-slate-800">
                <span>awaitingVerification</span><AppBadge tone="info">{{ overview.releases.awaitingVerification }}</AppBadge>
              </li>
              <li class="flex items-center justify-between gap-2 rounded-md bg-slate-50 px-3 py-1.5 dark:bg-slate-800">
                <span>integrityPassed</span><AppBadge tone="info">{{ overview.releases.integrityPassed }}</AppBadge>
              </li>
              <li class="flex items-center justify-between gap-2 rounded-md bg-slate-50 px-3 py-1.5 dark:bg-slate-800">
                <span>antivirusPassed</span><AppBadge tone="success">{{ overview.releases.antivirusPassed }}</AppBadge>
              </li>
              <li class="flex items-center justify-between gap-2 rounded-md bg-slate-50 px-3 py-1.5 dark:bg-slate-800">
                <span>reviewApproved</span><AppBadge tone="success">{{ overview.releases.reviewApproved }}</AppBadge>
              </li>
              <li class="flex items-center justify-between gap-2 rounded-md bg-slate-50 px-3 py-1.5 dark:bg-slate-800">
                <span>reviewRejected</span><AppBadge tone="warn">{{ overview.releases.reviewRejected }}</AppBadge>
              </li>
              <li class="flex items-center justify-between gap-2 rounded-md bg-slate-50 px-3 py-1.5 dark:bg-slate-800">
                <span>rejected</span><AppBadge tone="danger">{{ overview.releases.rejected }}</AppBadge>
              </li>
            </ul>
          </AppCard>
          <AppCard :title="t('dashboard.recentAudit')" label="recent-audit">
            <ul class="divide-y divide-slate-100 text-sm dark:divide-slate-800">
              <li v-for="e in audit?.events.slice(0, 8) ?? []" :key="e.id" class="flex flex-wrap gap-x-3 py-1.5">
                <span class="font-mono text-slate-800 dark:text-slate-200">{{ e.action }}</span>
                <span class="ml-auto text-slate-500 tabular-nums dark:text-slate-400">{{ e.createdAt }}</span>
              </li>
            </ul>
            <p v-if="(audit?.events.length ?? 0) === 0" class="mt-2 text-sm text-slate-500 dark:text-slate-400">{{ t('states.empty') }}</p>
            <AppButton size="sm" variant="ghost" icon="arrowRight" to="/app/audit" class="mt-2">
              {{ t('app.audit') }}
            </AppButton>
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
import StateBlock from '../components/StateBlock.vue'
import AppIcon from '../components/icons/AppIcon.vue'
import AppBadge from '../components/ui/AppBadge.vue'
import AppButton from '../components/ui/AppButton.vue'
import AppCard from '../components/ui/AppCard.vue'
import PageHeader from '../components/ui/PageHeader.vue'
import StatCard from '../components/ui/StatCard.vue'

const { t } = useI18n()
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

function refetchAll() {
  void refetchOv()
  void refetchAudit()
  void refetchQueue()
}
</script>
