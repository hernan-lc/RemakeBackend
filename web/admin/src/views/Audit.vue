<template>
  <div class="flex flex-col gap-4">
    <h1 class="text-xl font-bold">{{ t('audit.title') }}</h1>
    <StateBlock :loading="isPending" :error="!!error" :detail="errorDetail" :retry="true" @retry="refetch">
      <table v-if="!forbidden" class="w-full overflow-hidden rounded border border-slate-200 bg-white text-left text-sm">
        <thead>
          <tr class="border-b border-slate-200 bg-slate-50">
            <th scope="col" class="px-3 py-2">{{ t('audit.when') }}</th>
            <th scope="col" class="px-3 py-2">{{ t('audit.action') }}</th>
            <th scope="col" class="px-3 py-2">{{ t('audit.actor') }}</th>
            <th scope="col" class="px-3 py-2">{{ t('audit.target') }}</th>
          </tr>
        </thead>
        <tbody>
          <tr v-for="e in events" :key="e.id" class="border-b border-slate-100 last:border-0">
            <td class="whitespace-nowrap px-3 py-2 tabular-nums text-slate-600">{{ e.createdAt }}</td>
            <td class="px-3 py-2 font-mono">{{ e.action }}</td>
            <td class="break-all px-3 py-2 font-mono text-xs">{{ e.actorUserId }}</td>
            <td class="break-all px-3 py-2 font-mono text-xs">{{ e.targetUserId ?? e.publisherId ?? e.subjectId ?? '—' }}</td>
          </tr>
        </tbody>
      </table>
      <div v-if="forbidden" role="alert" class="rounded border border-amber-200 bg-amber-50 p-4 text-sm text-amber-800">
        {{ t('errors.notAdmin') }}
      </div>
      <p v-if="!forbidden && events.length === 0" class="mt-3 text-sm text-slate-500">{{ t('states.empty') }}</p>
      <button v-if="data?.nextCursor" type="button" class="mt-3 rounded border border-slate-300 px-3 py-1.5 text-sm font-medium hover:bg-slate-50" @click="cursor = data.nextCursor ?? undefined">
        {{ t('states.next') }}
      </button>
    </StateBlock>
  </div>
</template>

<script setup lang="ts">
import { computed, ref } from 'vue'
import { useI18n } from 'vue-i18n'
import { useQuery } from '@tanstack/vue-query'
import { ApiError, api } from '../api/client'
import StateBlock from '../components/StateBlock.vue'

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
