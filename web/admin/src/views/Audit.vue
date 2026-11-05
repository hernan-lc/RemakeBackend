<template>
  <div class="flex flex-col gap-4">
    <PageHeader :title="t('audit.title')" />
    <AppToolbar v-model="search" :search-label="t('audit.searchPage')" :refresh-label="t('states.refresh')" @refresh="refetch" />

    <StateBlock :loading="isPending" :error="!!error && !forbidden" :detail="errorDetail" :retry="true" @retry="refetch">
      <AppNotice v-if="forbidden" tone="warn">{{ t('errors.notAdmin') }}</AppNotice>
      <AppCard v-else-if="filteredEvents.length" :padded="false" label="audit-events">
        <div class="scroll-thin hidden overflow-x-auto md:block">
          <table class="w-full table-fixed text-left text-sm">
            <thead class="border-b border-slate-200 bg-slate-50/70 text-xs font-semibold uppercase tracking-wide text-slate-500 dark:border-slate-800 dark:bg-slate-800/40 dark:text-slate-400">
              <tr>
                <th scope="col" class="w-[23%] px-4 py-2.5">{{ t('audit.when') }}</th>
                <th scope="col" class="w-[29%] px-4 py-2.5">{{ t('audit.action') }}</th>
                <th scope="col" class="w-[24%] px-4 py-2.5">{{ t('audit.actor') }}</th>
                <th scope="col" class="w-[24%] px-4 py-2.5">{{ t('audit.target') }}</th>
              </tr>
            </thead>
            <tbody class="divide-y divide-slate-100 dark:divide-slate-800">
              <tr v-for="e in filteredEvents" :key="e.id" class="transition-colors hover:bg-slate-50 dark:hover:bg-slate-800/50">
                <td class="px-4 py-2.5 text-xs tabular-nums text-slate-600 dark:text-slate-400">{{ formatDateTime(e.createdAt, locale) }}</td>
                <td class="px-4 py-2.5"><code class="break-all text-xs font-semibold text-indigo-700 dark:text-indigo-300">{{ e.action }}</code></td>
                <td class="px-4 py-2.5"><code class="text-xs text-slate-700 dark:text-slate-300" :title="e.actorUserId">{{ compactId(e.actorUserId) }}</code></td>
                <td class="px-4 py-2.5"><code class="text-xs text-slate-700 dark:text-slate-300" :title="targetOf(e)">{{ compactId(targetOf(e)) }}</code></td>
              </tr>
            </tbody>
          </table>
        </div>
        <ul class="divide-y divide-slate-100 md:hidden dark:divide-slate-800">
          <li v-for="e in filteredEvents" :key="e.id" class="space-y-3 p-4">
            <div class="flex items-start gap-3">
                            <div class="min-w-0 flex-1">
                <p class="break-all font-mono text-xs font-semibold text-slate-900 dark:text-white">{{ e.action }}</p>
                <p class="mt-1 text-xs tabular-nums text-slate-500 dark:text-slate-400">{{ formatDateTime(e.createdAt, locale) }}</p>
              </div>
            </div>
            <dl class="grid grid-cols-2 gap-3 rounded-xl bg-slate-50 p-3 dark:bg-slate-800/60">
              <div class="min-w-0">
                <dt class="text-[11px] text-slate-500 dark:text-slate-400">{{ t('audit.actor') }}</dt>
                <dd class="mt-1 truncate font-mono text-xs" :title="e.actorUserId">{{ compactId(e.actorUserId) }}</dd>
              </div>
              <div class="min-w-0">
                <dt class="text-[11px] text-slate-500 dark:text-slate-400">{{ t('audit.target') }}</dt>
                <dd class="mt-1 truncate font-mono text-xs" :title="targetOf(e)">{{ compactId(targetOf(e)) }}</dd>
              </div>
            </dl>
          </li>
        </ul>
      </AppCard>
      <EmptyState v-if="!forbidden && filteredEvents.length === 0" :title="search ? t('states.noMatches') : t('states.empty')" icon="audit" />
      <div v-if="!forbidden && data?.nextCursor" class="mt-2 flex justify-center">
        <AppButton variant="secondary" icon="chevronDown" @click="cursor = data.nextCursor ?? undefined">{{ t('states.next') }}</AppButton>
      </div>
    </StateBlock>
  </div>
</template>

<script setup lang="ts">
import { computed, ref } from 'vue'
import { useI18n } from 'vue-i18n'
import { useQuery } from '@tanstack/vue-query'
import { ApiError, api } from '../api/client'
import { compactId, formatDateTime } from '../utils/format'
import type { AuditEventView } from '../api/types'
import StateBlock from '../components/StateBlock.vue'
import AppNotice from '../components/ui/AppNotice.vue'
import AppButton from '../components/ui/AppButton.vue'
import AppCard from '../components/ui/AppCard.vue'
import AppToolbar from '../components/ui/AppToolbar.vue'
import EmptyState from '../components/ui/EmptyState.vue'
import PageHeader from '../components/ui/PageHeader.vue'

const { t, locale } = useI18n()
const cursor = ref<string | undefined>(undefined)
const search = ref('')

const { data, isPending, error, refetch } = useQuery({
  queryKey: computed(() => ['admin', 'audit', cursor.value]),
  queryFn: () => api.audit({ cursor: cursor.value, limit: 50 }),
})
const events = computed(() => data.value?.events ?? [])
const targetOf = (e: AuditEventView) => e.targetUserId ?? e.publisherId ?? e.subjectId ?? ''
const filteredEvents = computed(() => {
  const query = search.value.trim().toLowerCase()
  return query ? events.value.filter((e) =>
    [e.action, e.actorUserId, targetOf(e), e.createdAt].some((part) => part.toLowerCase().includes(query))
  ) : events.value
})
const forbidden = computed(() => error.value instanceof ApiError && error.value.status === 403)
const errorDetail = computed(() =>
  error.value instanceof ApiError && error.value.status !== 403 ? `HTTP ${error.value.status}` : undefined,
)
</script>
