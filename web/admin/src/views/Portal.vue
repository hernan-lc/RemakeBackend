<template>
  <div class="flex flex-col gap-4">
    <PageHeader :title="t('portal.title')" :description="t('portal.releases')" icon="portal" />
    <StateBlock :loading="isPending" :error="!!error" :retry="true" @retry="refetch">
      <ul class="grid grid-cols-1 gap-3 xl:grid-cols-2">
        <li v-for="p in publishers" :key="p.id">
          <AppCard :label="p.namespace">
            <div class="flex flex-wrap items-center gap-x-3 gap-y-1">
              <span class="break-all font-mono text-sm font-semibold">{{ p.namespace }}</span>
              <AppBadge :tone="p.status === 'approved' ? 'success' : p.status === 'pending' ? 'warn' : 'neutral'">{{ p.status }}</AppBadge>
              <AppBadge tone="info" icon="account">{{ p.role }}</AppBadge>
            </div>
            <div class="mt-3">
              <AppButton size="sm" variant="ghost" :icon="members[p.id] ? 'chevronDown' : 'portal'" @click="toggleMembers(p.id)">
                {{ t('portal.members') }}
              </AppButton>
              <ul v-if="members[p.id]" class="mt-2 divide-y divide-slate-100 rounded-md border border-slate-200 text-sm dark:divide-slate-800 dark:border-slate-700">
                <li v-for="m in members[p.id]" :key="m.user_id" class="flex items-center justify-between gap-2 px-3 py-1.5">
                  <span class="min-w-0 flex-1 truncate font-mono text-xs">{{ m.user_id }}</span>
                  <AppBadge tone="neutral">{{ m.role }}</AppBadge>
                </li>
              </ul>
            </div>
            <p class="mt-3 flex items-start gap-2 rounded-md border border-slate-200 bg-slate-50 p-3 text-sm text-slate-600 dark:border-slate-700 dark:bg-slate-800 dark:text-slate-400" role="note">
              <AppIcon name="lock" :size="16" class="mt-0.5 shrink-0" aria-hidden="true" />
              {{ t('portal.keysDisabled') }}
            </p>
            <p class="mt-2 text-xs text-slate-500 dark:text-slate-400">{{ t('portal.notOwnerAdmin') }}</p>
          </AppCard>
        </li>
      </ul>
      <EmptyState v-if="publishers.length === 0" :title="t('states.empty')" icon="portal" />
    </StateBlock>
  </div>
</template>

<script setup lang="ts">
import { computed, reactive } from 'vue'
import { useI18n } from 'vue-i18n'
import { useQuery } from '@tanstack/vue-query'
import { api } from '../api/client'
import { useUiStore } from '../stores/ui'
import type { PublisherMember } from '../api/types'
import StateBlock from '../components/StateBlock.vue'
import AppIcon from '../components/icons/AppIcon.vue'
import AppBadge from '../components/ui/AppBadge.vue'
import AppButton from '../components/ui/AppButton.vue'
import AppCard from '../components/ui/AppCard.vue'
import EmptyState from '../components/ui/EmptyState.vue'
import PageHeader from '../components/ui/PageHeader.vue'

const { t } = useI18n()
const ui = useUiStore()

const { data, isPending, error, refetch } = useQuery({
  queryKey: ['portal', 'publishers'],
  queryFn: api.myPublishers,
})
const publishers = computed(() => data.value ?? [])
const members = reactive<Record<string, PublisherMember[]>>({})

async function toggleMembers(id: string) {
  if (members[id]) {
    delete members[id]
    return
  }
  try {
    members[id] = await api.publisherMembers(id)
  } catch {
    ui.toast(t('errors.loadFailed'), 'err')
  }
}
</script>
