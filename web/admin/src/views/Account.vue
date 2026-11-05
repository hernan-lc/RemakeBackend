<template>
  <div class="flex flex-col gap-5">
    <PageHeader :title="t('app.account')" />
    <StateBlock :loading="isPending" :error="!!error" :retry="true" @retry="refetch">
      <AppCard v-if="me" label="account-details">
        <div class="flex flex-wrap items-center gap-3">
          <span class="flex size-10 shrink-0 items-center justify-center rounded-full bg-indigo-100 font-semibold text-indigo-700 dark:bg-indigo-500/15 dark:text-indigo-300">{{ me.name?.charAt(0).toUpperCase() || 'A' }}</span>
          <div class="min-w-0 flex-1">
            <p class="font-semibold">{{ me.name }}</p>
            <p class="break-all text-xs text-slate-500 dark:text-slate-400">{{ me.email }}</p>
          </div>
          <div class="flex flex-wrap gap-1">
            <AppBadge v-for="role in me.roles" :key="role" tone="info">{{ role }}</AppBadge>
          </div>
        </div>
      </AppCard>
    </StateBlock>

    <div class="grid grid-cols-1 items-start gap-4 xl:grid-cols-2">
      <AppCard label="linked-providers" :title="t('auth.linkedProviders')">
        <StateBlock :loading="providersPending" :error="providersFailed" :retry="true" @retry="refetchProviders">
          <ul v-if="providers" class="divide-y divide-slate-100 dark:divide-slate-800">
            <li v-for="provider in providers" :key="provider.providerId" class="flex flex-wrap items-center gap-2 py-3 text-sm first:pt-0 last:pb-0">
              <AppBadge tone="neutral">{{ provider.providerId }}</AppBadge>
              <span class="min-w-0 flex-1 break-words text-xs text-slate-500 dark:text-slate-400">{{ provider.accountId }}</span>
              <AppButton v-if="providers.length > 1" variant="secondary" size="sm"
                :loading="unlinking === provider.providerId"
                @click="confirming === provider.providerId ? unlink(provider.providerId) : (confirming = provider.providerId)">
                {{ confirming === provider.providerId ? t('states.confirm') : t('auth.unlink') }}
              </AppButton>
            </li>
          </ul>
          <p v-if="unlinkFailed" role="alert" class="mt-2 text-sm text-red-700 dark:text-red-300">{{ t('errors.actionFailed') }}</p>
        </StateBlock>
      </AppCard>
      <AppCard label="set-username" :title="t('auth.setUsername')">
        <form class="flex flex-col gap-3 sm:flex-row sm:items-end" @submit.prevent="saveUsername">
          <AppInput v-model="username" :label="t('auth.username')" type="text" autocomplete="username" class="flex-1" />
          <AppButton type="submit" variant="secondary" :loading="savingUsername">{{ t('auth.setUsername') }}</AppButton>
        </form>
        <p class="mt-2 text-xs leading-5 text-slate-500 dark:text-slate-400">{{ t('auth.usernameNote') }}</p>
        <p v-if="usernameFailed" role="alert" class="mt-2 text-sm text-red-700 dark:text-red-300">{{ t('errors.actionFailed') }}</p>
        <p v-if="usernameSaved" role="status" class="mt-2 text-sm text-green-700 dark:text-green-300">{{ t('auth.usernameSaved') }}</p>
      </AppCard>
    </div>
    <div><AppButton variant="ghost" icon="logout" to="/logout">{{ t('app.logout') }}</AppButton></div>
  </div>
</template>

<script setup lang="ts">
import { onMounted, ref } from 'vue'
import { useI18n } from 'vue-i18n'
import { api, ApiError } from '../api/client'
import type { LinkedProvider } from '../api/types'
import { useSession } from '../queries/session'
import StateBlock from '../components/StateBlock.vue'
import AppBadge from '../components/ui/AppBadge.vue'
import AppButton from '../components/ui/AppButton.vue'
import AppCard from '../components/ui/AppCard.vue'
import AppInput from '../components/ui/AppInput.vue'
import PageHeader from '../components/ui/PageHeader.vue'

const { t } = useI18n()
const { data: me, isPending, error, refetch } = useSession()

const providers = ref<LinkedProvider[] | null>(null)
const providersPending = ref(false)
const providersFailed = ref(false)
const confirming = ref<string | null>(null)
const unlinking = ref<string | null>(null)
const unlinkFailed = ref(false)

async function refetchProviders() {
  providersPending.value = true
  providersFailed.value = false
  unlinkFailed.value = false
  confirming.value = null
  try {
    providers.value = await api.linkedProviders()
  } catch (e) {
    providersFailed.value = e instanceof ApiError
  } finally {
    providersPending.value = false
  }
}

async function unlink(providerId: string) {
  unlinking.value = providerId
  unlinkFailed.value = false
  try {
    await api.unlinkProvider(providerId)
    await refetchProviders()
  } catch (e) {
    unlinkFailed.value = e instanceof ApiError
  } finally {
    unlinking.value = null
  }
}

const username = ref('')
const savingUsername = ref(false)
const usernameFailed = ref(false)
const usernameSaved = ref(false)

async function saveUsername() {
  savingUsername.value = true
  usernameFailed.value = false
  usernameSaved.value = false
  try {
    await api.setUsername(username.value.trim())
    usernameSaved.value = true
  } catch (e) {
    usernameFailed.value = e instanceof ApiError
  } finally {
    savingUsername.value = false
  }
}

onMounted(refetchProviders)
</script>
