<template>
  <div class="flex flex-col gap-4">
    <PageHeader :title="t('app.account')" icon="account" />
    <StateBlock :loading="isPending" :error="!!error" :retry="true" @retry="refetch">
      <AppCard v-if="me" label="account-details">
        <dl class="flex flex-col gap-3 text-sm">
          <div class="flex flex-col gap-1 sm:flex-row sm:items-center sm:gap-3">
            <dt class="w-28 shrink-0 font-medium text-slate-500 dark:text-slate-400">{{ t('auth.signedInAs') }}</dt>
            <dd class="min-w-0 flex-1 break-words font-medium">{{ me.name }} ({{ me.email }})</dd>
          </div>
          <div class="flex flex-col gap-1 sm:flex-row sm:items-center sm:gap-3">
            <dt class="w-28 shrink-0 font-medium text-slate-500 dark:text-slate-400">{{ t('auth.roles') }}</dt>
            <dd class="flex flex-wrap gap-1.5">
              <AppBadge v-for="role in me.roles" :key="role" tone="info" icon="account">{{ role }}</AppBadge>
              <span v-if="me.roles.length === 0" class="text-slate-500 dark:text-slate-400">—</span>
            </dd>
          </div>
        </dl>
      </AppCard>
    </StateBlock>
    <div>
      <AppButton variant="secondary" icon="logout" to="/logout">
        {{ t('app.logout') }}
      </AppButton>
    </div>
  </div>
</template>

<script setup lang="ts">
import { useI18n } from 'vue-i18n'
import { useSession } from '../queries/session'
import StateBlock from '../components/StateBlock.vue'
import AppBadge from '../components/ui/AppBadge.vue'
import AppButton from '../components/ui/AppButton.vue'
import AppCard from '../components/ui/AppCard.vue'
import PageHeader from '../components/ui/PageHeader.vue'

const { t } = useI18n()
const { data: me, isPending, error, refetch } = useSession()
</script>
