<template>
  <div class="flex flex-col gap-6">
    <h1 class="text-xl font-bold">{{ t('dashboard.title') }}</h1>
    <StateBlock :loading="ovPending || auditPending || queuePending" :error="!!ovError" :retry="true" @retry="refetchAll">
      <div v-if="!isAdmin" role="alert" class="rounded border border-amber-200 bg-amber-50 p-4 text-sm text-amber-800">
        {{ t('errors.notAdmin') }}
      </div>
      <template v-else>
        <dl class="grid grid-cols-1 gap-3 sm:grid-cols-2 lg:grid-cols-4">
          <div class="rounded border border-slate-200 bg-white p-4">
            <dt class="text-sm text-slate-600">{{ t('dashboard.pendingPublishers') }}</dt>
            <dd class="mt-1 text-3xl font-bold tabular-nums">{{ overview?.publishers.pending ?? 0 }}</dd>
          </div>
          <div class="rounded border border-slate-200 bg-white p-4">
            <dt class="text-sm text-slate-600">{{ t('dashboard.reviewQueue') }}</dt>
            <dd class="mt-1 text-3xl font-bold tabular-nums">{{ queue?.items.length ?? 0 }}</dd>
          </div>
          <div class="rounded border border-slate-200 bg-white p-4">
            <dt class="text-sm text-slate-600">{{ t('dashboard.pendingKeys') }}</dt>
            <dd class="mt-1 text-3xl font-bold tabular-nums">{{ overview?.pendingKeys ?? 0 }}</dd>
          </div>
          <div class="rounded border border-slate-200 bg-white p-4">
            <dt class="text-sm text-slate-600">{{ t('dashboard.auditEvents') }}</dt>
            <dd class="mt-1 text-3xl font-bold tabular-nums">{{ overview?.auditEvents ?? 0 }}</dd>
          </div>
        </dl>
        <section aria-label="releases" class="mt-6 rounded border border-slate-200 bg-white p-4">
          <h2 class="text-sm font-semibold">{{ t('dashboard.releases') }}</h2>
          <ul v-if="overview" class="mt-2 grid grid-cols-2 gap-1 text-sm tabular-nums text-slate-700 sm:grid-cols-3">
            <li>awaitingVerification: {{ overview.releases.awaitingVerification }}</li>
            <li>integrityPassed: {{ overview.releases.integrityPassed }}</li>
            <li>antivirusPassed: {{ overview.releases.antivirusPassed }}</li>
            <li>reviewApproved: {{ overview.releases.reviewApproved }}</li>
            <li>reviewRejected: {{ overview.releases.reviewRejected }}</li>
            <li>rejected: {{ overview.releases.rejected }}</li>
          </ul>
        </section>
        <section aria-label="recent-audit" class="mt-6 rounded border border-slate-200 bg-white p-4">
          <h2 class="text-sm font-semibold">{{ t('dashboard.recentAudit') }}</h2>
          <ul class="mt-2 divide-y divide-slate-100 text-sm">
            <li v-for="e in audit?.events.slice(0, 8) ?? []" :key="e.id" class="flex flex-wrap gap-x-3 py-1.5">
              <span class="font-mono text-slate-800">{{ e.action }}</span>
              <span class="text-slate-500">{{ e.createdAt }}</span>
            </li>
          </ul>
          <RouterLink to="/app/audit" class="mt-2 inline-block text-sm font-medium underline">{{ t('app.audit') }}</RouterLink>
        </section>
      </template>
    </StateBlock>
  </div>
</template>

<script setup lang="ts">
import { computed } from 'vue'
import { useI18n } from 'vue-i18n'
import { RouterLink } from 'vue-router'
import { useQuery } from '@tanstack/vue-query'
import { api } from '../api/client'
import { isCloudAdmin } from '../api/types'
import { useSession } from '../queries/session'
import StateBlock from '../components/StateBlock.vue'

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
