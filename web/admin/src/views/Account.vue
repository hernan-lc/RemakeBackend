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
    <AppCard label="linked-providers">
      <h2 class="mb-1 text-sm font-semibold text-slate-900 dark:text-white">{{ t('auth.linkedProviders') }}</h2>
      <p class="mb-3 text-sm text-slate-600 dark:text-slate-400">{{ t('auth.googleLinkNote') }}</p>
      <StateBlock :loading="providersPending" :error="providersFailed" :retry="true" @retry="refetchProviders">
        <ul v-if="providers" class="flex flex-col gap-2">
          <li v-for="provider in providers" :key="provider.providerId" class="flex items-center gap-2 text-sm">
            <AppBadge tone="info" icon="account">{{ provider.providerId }}</AppBadge>
            <span class="min-w-0 flex-1 break-words text-slate-500 dark:text-slate-400">{{ provider.accountId }}</span>
            <AppButton
              v-if="providers.length > 1"
              variant="secondary"
              size="sm"
              :loading="unlinking === provider.providerId"
              @click="confirming === provider.providerId ? unlink(provider.providerId) : (confirming = provider.providerId)"
            >
              {{ confirming === provider.providerId ? t('states.confirm') : t('auth.unlink') }}
            </AppButton>
          </li>
        </ul>
        <p v-if="unlinkFailed" role="alert" class="mt-2 text-sm text-red-700 dark:text-red-300">
          {{ t('errors.actionFailed') }}
        </p>
      </StateBlock>
    </AppCard>
    <AppCard label="set-username">
      <h2 class="mb-1 text-sm font-semibold text-slate-900 dark:text-white">{{ t('auth.setUsername') }}</h2>
      <p class="mb-3 text-sm text-slate-600 dark:text-slate-400">{{ t('auth.usernameNote') }}</p>
      <form class="flex flex-col gap-3 sm:flex-row sm:items-end" @submit.prevent="saveUsername">
        <AppInput v-model="username" :label="t('auth.username')" type="text" autocomplete="username" icon="account" class="flex-1" />
        <AppButton type="submit" variant="secondary" :loading="savingUsername">
          {{ t('auth.setUsername') }}
        </AppButton>
      </form>
      <p v-if="usernameFailed" role="alert" class="mt-2 text-sm text-red-700 dark:text-red-300">
        {{ t('errors.actionFailed') }}
      </p>
      <p v-if="usernameSaved" role="status" class="mt-2 text-sm text-green-700 dark:text-green-300">
        {{ t('auth.usernameSaved') }}
      </p>
    </AppCard>
    <div>
      <AppButton variant="secondary" icon="logout" to="/logout">
        {{ t('app.logout') }}
      </AppButton>
    </div>
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
