<template>
  <div class="flex flex-col gap-5">
    <PageHeader :title="t('app.account')" :description="copy.description" />
    <div class="grid items-start gap-5 xl:grid-cols-[minmax(0,0.8fr)_minmax(0,1.6fr)]">
    <StateBlock :loading="isPending" :error="!!error" :retry="true" @retry="refetch">
      <AppCard v-if="me" label="account-details">
        <p class="mb-5 text-xs font-semibold uppercase tracking-wider text-slate-600 dark:text-slate-300">{{ copy.profile }}</p>
        <div class="flex flex-col items-start gap-4">
          <span class="flex size-12 shrink-0 items-center justify-center rounded-full bg-indigo-100 font-semibold text-indigo-700 dark:bg-indigo-500/15 dark:text-indigo-300">{{ me.name?.charAt(0).toUpperCase() || 'A' }}</span>
          <div class="min-w-0 flex-1">
            <h2 class="text-lg font-semibold">{{ me.name }}</h2>
            <p class="break-all text-xs text-slate-600 dark:text-slate-300">{{ me.email }}</p>
          </div>
          <div class="flex flex-wrap gap-1">
            <AppBadge v-for="role in me.roles" :key="role" tone="info">{{ role }}</AppBadge>
          </div>
        </div>
        <div class="mt-6 border-t border-slate-200 pt-4 dark:border-slate-700"><AppButton variant="secondary" icon="logout" to="/logout">{{ t('app.logout') }}</AppButton></div>
      </AppCard>
    </StateBlock>

    <div class="flex min-w-0 flex-col gap-5">
      <div><h2 class="text-lg font-semibold">{{ copy.access }}</h2><p class="mt-1 text-sm text-slate-600 dark:text-slate-300">{{ copy.accessHint }}</p></div>
      <AppCard label="linked-providers" :title="t('auth.linkedProviders')">
        <StateBlock :loading="providersPending" :error="providersFailed" :retry="true" @retry="refetchProviders">
          <ul v-if="providers?.length" class="divide-y divide-slate-100 dark:divide-slate-800">
            <li v-for="provider in providers" :key="provider.providerId" class="flex flex-wrap items-center gap-2 py-3 text-sm first:pt-0 last:pb-0">
              <AppBadge tone="neutral">{{ provider.providerId }}</AppBadge>
              <span class="min-w-0 flex-1 break-words text-xs text-slate-600 dark:text-slate-300">{{ provider.accountId }}</span>
              <AppButton v-if="providers.length > 1" variant="secondary" size="sm"
                :loading="unlinking === provider.providerId" :disabled="!!unlinking"
                @click="confirming === provider.providerId ? unlink(provider.providerId) : (confirming = provider.providerId)">
                {{ confirming === provider.providerId ? t('states.confirm') : t('auth.unlink') }}
              </AppButton>
              <AppButton v-if="confirming === provider.providerId" size="sm" variant="ghost" :disabled="!!unlinking" @click="confirming = null">{{ t('states.cancel') }}</AppButton>
            </li>
          </ul>
          <p v-else class="py-3 text-sm text-slate-600 dark:text-slate-300">{{ copy.noProviders }}</p>
          <p v-if="unlinkFailed" role="alert" class="mt-2 text-sm text-red-700 dark:text-red-300">{{ t('errors.actionFailed') }}</p>
        </StateBlock>
      </AppCard>
      <AppCard label="set-username" :title="t('auth.setUsername')">
        <form class="flex flex-col gap-3 sm:flex-row sm:items-end" @submit.prevent="saveUsername">
          <AppInput v-model="username" :label="t('auth.username')" type="text" autocomplete="username" class="flex-1" />
          <AppButton type="submit" variant="secondary" :loading="savingUsername">{{ t('auth.setUsername') }}</AppButton>
        </form>
        <p class="mt-2 text-xs leading-5 text-slate-600 dark:text-slate-300">{{ t('auth.usernameNote') }}</p>
        <p v-if="usernameFailed" role="alert" class="mt-2 text-sm text-red-700 dark:text-red-300">{{ t('errors.actionFailed') }}</p>
        <p v-if="usernameSaved" role="status" class="mt-2 text-sm text-green-700 dark:text-green-300">{{ t('auth.usernameSaved') }}</p>
      </AppCard>
    </div>
    </div>
  </div>
</template>

<script setup lang="ts">
import { computed, onMounted, ref } from 'vue'
import { useI18n } from 'vue-i18n'
import { api } from '../api/client'
import type { LinkedProvider } from '../api/types'
import { useSession } from '../queries/session'
import StateBlock from '../components/StateBlock.vue'
import AppBadge from '../components/ui/AppBadge.vue'
import AppButton from '../components/ui/AppButton.vue'
import AppCard from '../components/ui/AppCard.vue'
import AppInput from '../components/ui/AppInput.vue'
import PageHeader from '../components/ui/PageHeader.vue'

const { t, locale } = useI18n()
const copy = computed(() => locale.value === 'es' ? {
  description: 'Gestiona tu identidad y las formas de acceder a Cloud.', profile: 'Tu perfil', access: 'Acceso y seguridad',
  accessHint: 'Revisa tus métodos vinculados y el identificador de tu cuenta.', noProviders: 'No hay métodos vinculados para mostrar.',
} : {
  description: 'Manage your identity and the ways you access Cloud.', profile: 'Your profile', access: 'Access and security',
  accessHint: 'Review linked sign-in methods and your account identifier.', noProviders: 'No linked sign-in methods to display.',
})
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
  } catch {
    providersFailed.value = true
  } finally {
    providersPending.value = false
  }
}

async function unlink(providerId: string) {
  if (unlinking.value) return
  unlinking.value = providerId
  unlinkFailed.value = false
  try {
    await api.unlinkProvider(providerId)
    await refetchProviders()
  } catch {
    unlinkFailed.value = true
  } finally {
    unlinking.value = null
  }
}

const username = ref('')
const savingUsername = ref(false)
const usernameFailed = ref(false)
const usernameSaved = ref(false)

async function saveUsername() {
  if (savingUsername.value) return
  savingUsername.value = true
  usernameFailed.value = false
  usernameSaved.value = false
  try {
    await api.setUsername(username.value.trim())
    usernameSaved.value = true
  } catch {
    usernameFailed.value = true
  } finally {
    savingUsername.value = false
  }
}

onMounted(refetchProviders)
</script>
