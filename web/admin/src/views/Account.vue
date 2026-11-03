<template>
  <div class="flex flex-col gap-4">
    <h1 class="text-xl font-bold">{{ t('app.account') }}</h1>
    <StateBlock :loading="isPending" :error="!!error" :retry="true" @retry="refetch">
      <dl v-if="me" class="rounded border border-slate-200 bg-white p-4 text-sm">
        <div class="flex gap-2 py-1"><dt class="w-24 shrink-0 text-slate-500">{{ t('auth.signedInAs') }}</dt><dd class="font-medium">{{ me.name }} ({{ me.email }})</dd></div>
        <div class="flex gap-2 py-1"><dt class="w-24 shrink-0 text-slate-500">{{ t('auth.roles') }}</dt><dd class="font-mono">{{ me.roles.join(', ') || '—' }}</dd></div>
      </dl>
    </StateBlock>
    <div>
      <RouterLink to="/logout" class="rounded bg-slate-900 px-4 py-2 text-sm font-medium text-white hover:bg-slate-700">
        {{ t('app.logout') }}
      </RouterLink>
    </div>
  </div>
</template>

<script setup lang="ts">
import { useI18n } from 'vue-i18n'
import { RouterLink } from 'vue-router'
import { useSession } from '../queries/session'
import StateBlock from '../components/StateBlock.vue'

const { t } = useI18n()
const { data: me, isPending, error, refetch } = useSession()
</script>
