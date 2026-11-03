<template>
  <div class="flex flex-col gap-4">
    <h1 class="text-xl font-bold">{{ t('portal.title') }}</h1>
    <p class="text-sm text-slate-600">{{ t('portal.releases') }}</p>
    <StateBlock :loading="isPending" :error="!!error" :retry="true" @retry="refetch">
      <ul class="flex flex-col gap-3">
        <li v-for="p in publishers" :key="p.id" class="rounded border border-slate-200 bg-white p-4">
          <div class="flex flex-wrap items-baseline gap-x-3">
            <span class="font-mono text-sm font-semibold">{{ p.namespace }}</span>
            <span class="text-xs text-slate-500">{{ p.status }} · {{ p.role }}</span>
          </div>
          <div class="mt-2">
            <button type="button" class="text-sm font-medium underline" @click="toggleMembers(p.id)">
              {{ t('portal.members') }}
            </button>
            <ul v-if="members[p.id]" class="mt-1 text-sm text-slate-700">
              <li v-for="m in members[p.id]" :key="m.user_id" class="font-mono text-xs">{{ m.user_id }} — {{ m.role }}</li>
            </ul>
          </div>
          <p class="mt-3 rounded border border-slate-200 bg-slate-50 p-3 text-sm text-slate-600" role="note">
            {{ t('portal.keysDisabled') }}
          </p>
          <p class="mt-2 text-xs text-slate-500">{{ t('portal.notOwnerAdmin') }}</p>
        </li>
      </ul>
      <p v-if="publishers.length === 0" class="mt-3 text-sm text-slate-500">{{ t('states.empty') }}</p>
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
