<template>
  <div class="flex flex-col gap-4">
    <PageHeader :title="t('audit.title')" icon="audit">
      <template #actions>
        <AppButton size="sm" variant="secondary" icon="refresh" icon-only :label="t('states.refresh')" :title="t('states.refresh')" @click="refetch" />
      </template>
    </PageHeader>
    <StateBlock :loading="isPending" :error="!!error" :detail="errorDetail" :retry="true" @retry="refetch">
      <AppCard v-if="!forbidden" :padded="false" label="audit-events">
        <div class="scroll-thin overflow-x-auto">
          <table class="w-full min-w-[40rem] text-left text-sm">
            <thead>
              <tr class="border-b border-slate-200 bg-slate-50 dark:border-slate-700 dark:bg-slate-800">
                <th scope="col" class="px-4 py-2.5 font-semibold">{{ t('audit.when') }}</th>
                <th scope="col" class="px-4 py-2.5 font-semibold">{{ t('audit.action') }}</th>
                <th scope="col" class="px-4 py-2.5 font-semibold">{{ t('audit.actor') }}</th>
                <th scope="col" class="px-4 py-2.5 font-semibold">{{ t('audit.target') }}</th>
              </tr>
            </thead>
            <tbody class="divide-y divide-slate-100 dark:divide-slate-800">
              <tr v-for="e in events" :key="e.id">
                <td class="whitespace-nowrap px-4 py-2.5 text-slate-600 tabular-nums dark:text-slate-400">{{ e.createdAt }}</td>
                <td class="px-4 py-2.5 font-mono">{{ e.action }}</td>
                <td class="max-w-48 truncate px-4 py-2.5 font-mono text-xs" :title="e.actorUserId">{{ e.actorUserId }}</td>
                <td class="max-w-48 truncate px-4 py-2.5 font-mono text-xs" :title="e.targetUserId ?? e.publisherId ?? e.subjectId ?? ''">
                  {{ e.targetUserId ?? e.publisherId ?? e.subjectId ?? '—' }}
                </td>
              </tr>
            </tbody>
          </table>
        </div>
      </AppCard>
      <div v-if="forbidden" role="alert" class="flex items-start gap-2 rounded-lg border border-amber-200 bg-amber-50 p-4 text-sm text-amber-800 dark:border-amber-900 dark:bg-amber-950 dark:text-amber-200">
        <AppIcon name="alert" :size="17" class="mt-0.5 shrink-0" aria-hidden="true" />
        {{ t('errors.notAdmin') }}
      </div>
      <EmptyState v-if="!forbidden && events.length === 0" :title="t('states.empty')" icon="audit" />
      <div v-if="data?.nextCursor" class="mt-3 flex justify-center">
        <AppButton variant="secondary" icon="chevronDown" @click="cursor = data.nextCursor ?? undefined">
          {{ t('states.next') }}
        </AppButton>
      </div>
    </StateBlock>
  </div>
</template>

<script setup lang="ts">
import { computed, ref } from 'vue'
import { useI18n } from 'vue-i18n'
import { useQuery } from '@tanstack/vue-query'
import { ApiError, api } from '../api/client'
import StateBlock from '../components/StateBlock.vue'
import AppIcon from '../components/icons/AppIcon.vue'
import AppButton from '../components/ui/AppButton.vue'
import AppCard from '../components/ui/AppCard.vue'
import EmptyState from '../components/ui/EmptyState.vue'
import PageHeader from '../components/ui/PageHeader.vue'

const { t } = useI18n()
const cursor = ref<string | undefined>(undefined)

const { data, isPending, error, refetch } = useQuery({
  queryKey: computed(() => ['admin', 'audit', cursor.value]),
  queryFn: () => api.audit({ cursor: cursor.value, limit: 50 }),
})
const events = computed(() => data.value?.events ?? [])
const forbidden = computed(() => error.value instanceof ApiError && error.value.status === 403)
const errorDetail = computed(() =>
  error.value instanceof ApiError && error.value.status !== 403 ? `HTTP ${error.value.status}` : undefined,
)
</script>
