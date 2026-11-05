<template>
  <div class="flex flex-col gap-4">
    <PageHeader :title="t('audit.title')" icon="audit">
      <template #actions>
        <AppButton size="sm" variant="secondary" icon="refresh" icon-only :label="t('states.refresh')" :title="t('states.refresh')" @click="refetch" />
      </template>
    </PageHeader>
    <StateBlock :loading="isPending" :error="!!error && !forbidden" :detail="errorDetail" :retry="true" @retry="refetch">
      <AppCard v-if="!forbidden" :padded="false" label="audit-events">
        <div class="scroll-thin overflow-x-auto">
          <table class="block w-full text-left text-sm md:table">
            <thead class="sr-only md:not-sr-only md:table-header-group">
              <tr class="border-b border-slate-200 bg-slate-50 dark:border-slate-700 dark:bg-slate-800">
                <th scope="col" class="px-4 py-2.5 font-semibold">{{ t('audit.when') }}</th>
                <th scope="col" class="px-4 py-2.5 font-semibold">{{ t('audit.action') }}</th>
                <th scope="col" class="px-4 py-2.5 font-semibold">{{ t('audit.actor') }}</th>
                <th scope="col" class="px-4 py-2.5 font-semibold">{{ t('audit.target') }}</th>
              </tr>
            </thead>
            <tbody class="block divide-y divide-slate-100 md:table-row-group dark:divide-slate-800">
              <tr v-for="e in events" :key="e.id" class="block py-3 md:table-row md:py-0">
                <td class="block break-words px-4 py-1 text-slate-600 tabular-nums md:table-cell md:whitespace-nowrap md:py-2.5 dark:text-slate-400"><span class="mr-2 text-xs md:hidden">{{ t('audit.when') }}:</span>{{ e.createdAt }}</td>
                <td class="block break-all px-4 py-1 font-mono md:table-cell md:py-2.5"><span class="mr-2 font-sans text-xs text-slate-500 md:hidden">{{ t('audit.action') }}:</span>{{ e.action }}</td>
                <td class="block break-all px-4 py-1 font-mono text-xs md:table-cell md:max-w-48 md:truncate md:py-2.5" :title="e.actorUserId"><span class="mr-2 font-sans text-slate-500 md:hidden">{{ t('audit.actor') }}:</span>{{ e.actorUserId }}</td>
                <td class="block break-all px-4 py-1 font-mono text-xs md:table-cell md:max-w-48 md:truncate md:py-2.5" :title="e.targetUserId ?? e.publisherId ?? e.subjectId ?? ''">
                  <span class="mr-2 font-sans text-slate-500 md:hidden">{{ t('audit.target') }}:</span>
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
