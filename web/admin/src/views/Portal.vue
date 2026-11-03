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
            <div v-if="canEnroll(p)" class="mt-3 flex flex-col gap-2">
              <AppButton size="sm" variant="ghost" icon="key" @click="openEnroll(p.id)">
                {{ t('portal.enrollKey') }}
              </AppButton>
              <div v-if="enroll.publisherId === p.id" class="flex flex-col gap-2 rounded-md border border-slate-200 bg-slate-50 p-3 text-sm dark:border-slate-700 dark:bg-slate-800">
                <p class="text-slate-600 dark:text-slate-400">{{ t('portal.enrollIntro') }}</p>
                <template v-if="enroll.challenge">
                  <p class="text-slate-600 dark:text-slate-400">{{ t('portal.challengeReady') }}</p>
                  <dl class="grid grid-cols-[auto_1fr] gap-x-3 gap-y-1 font-mono text-xs">
                    <dt class="font-sans text-slate-500 dark:text-slate-400">{{ t('portal.challengeId') }}</dt>
                    <dd class="break-all">{{ enroll.challenge.challengeId }}</dd>
                    <dt class="font-sans text-slate-500 dark:text-slate-400">{{ t('portal.nonce') }}</dt>
                    <dd class="break-all">{{ enroll.challenge.nonce }}</dd>
                  </dl>
                  <p class="font-mono text-xs text-slate-500 dark:text-slate-400">{{ t('portal.cliHint') }}</p>
                  <form class="flex flex-col gap-2" @submit.prevent="submitKey(p.id)">
                    <AppInput v-model="enroll.keyId" :label="t('portal.keyId')" required />
                    <AppInput v-model="enroll.publicKey" :label="t('portal.publicKey')" required />
                    <AppInput v-model="enroll.signature" :label="t('portal.signature')" required />
                    <p v-if="enroll.error" role="alert" class="flex items-start gap-2 text-red-700 dark:text-red-300">
                      <AppIcon name="alert" :size="16" class="mt-0.5 shrink-0" aria-hidden="true" />
                      {{ enroll.error }}
                    </p>
                    <AppButton type="submit" size="sm" variant="primary" :loading="enroll.busy" :disabled="enroll.busy">
                      {{ t('portal.submitKey') }}
                    </AppButton>
                  </form>
                </template>
              </div>
            </div>
            <p v-else class="mt-3 text-xs text-slate-500 dark:text-slate-400">{{ t('portal.approvalsNote') }}</p>
            <p class="mt-2 text-xs text-slate-500 dark:text-slate-400">{{ t('portal.notOwnerAdmin') }}</p>
          </AppCard>
        </li>
      </ul>
      <EmptyState v-if="publishers.length === 0" :title="t('states.empty')" icon="portal" />
    </StateBlock>
    <StepUpDialog
      :open="dialogOpen"
      action="publisher.key.enroll"
      :action-label="t('portal.enrollKey')"
      :publisher-id="enroll.publisherId ?? ''"
      :return-to="dialogReturnTo"
      @granted="onGranted"
      @cancelled="dialogOpen = false"
    />
  </div>
</template>

<script setup lang="ts">
import { computed, onMounted, reactive, ref, watch } from 'vue'
import { useI18n } from 'vue-i18n'
import { useRoute, useRouter } from 'vue-router'
import { useQuery } from '@tanstack/vue-query'
import { ApiError, api } from '../api/client'
import { parseStepUpQuery } from '../api/auth'
import { useUiStore } from '../stores/ui'
import type { KeyChallenge, MyPublisher, PublisherMember } from '../api/types'
import StateBlock from '../components/StateBlock.vue'
import StepUpDialog from '../components/StepUpDialog.vue'
import AppIcon from '../components/icons/AppIcon.vue'
import AppBadge from '../components/ui/AppBadge.vue'
import AppButton from '../components/ui/AppButton.vue'
import AppCard from '../components/ui/AppCard.vue'
import AppInput from '../components/ui/AppInput.vue'
import EmptyState from '../components/ui/EmptyState.vue'
import PageHeader from '../components/ui/PageHeader.vue'

const { t } = useI18n()
const ui = useUiStore()
const route = useRoute()
const router = useRouter()

const { data, isPending, error, refetch } = useQuery({
  queryKey: ['portal', 'publishers'],
  queryFn: api.myPublishers,
})
const publishers = computed(() => data.value ?? [])
const members = reactive<Record<string, PublisherMember[]>>({})

function canEnroll(p: MyPublisher): boolean {
  return p.status === 'approved' && (p.role === 'owner' || p.role === 'admin')
}

// Single active enrollment: the dialog mints a session-bound grant, the
// challenge validates it, and registration spends it. Everything lives in
// component state; nothing touches web storage.
const enroll = reactive({
  publisherId: null as string | null,
  grantId: null as string | null,
  challenge: null as KeyChallenge | null,
  keyId: '',
  publicKey: '',
  signature: '',
  busy: false,
  error: '',
})
const dialogOpen = ref(false)
const dialogReturnTo = ref('/app/portal')

function openEnroll(publisherId: string) {
  enroll.publisherId = publisherId
  enroll.grantId = null
  enroll.challenge = null
  enroll.keyId = ''
  enroll.publicKey = ''
  enroll.signature = ''
  enroll.busy = false
  enroll.error = ''
  dialogReturnTo.value = `/app/portal?stepUp=enroll&publisher=${publisherId}`
  dialogOpen.value = true
}

function stepUpFailed(status: number, code?: string): string {
  if (status === 401) {
    void router.replace({ path: '/login', query: { next: '/app/portal' } })
    return t('errors.expired')
  }
  if (status === 403 && code === 'step_up_expired') return t('stepUp.expired')
  return t('errors.actionFailed')
}

async function fetchChallenge(publisherId: string, grantId: string) {
  enroll.busy = true
  enroll.error = ''
  try {
    enroll.challenge = await api.publisherKeyChallenge(publisherId, grantId)
  } catch (e) {
    if (e instanceof ApiError) {
      enroll.error = stepUpFailed(e.status, e.code)
      if (e.status === 403 && e.code === 'step_up_expired') {
        dialogReturnTo.value = `/app/portal?stepUp=enroll&publisher=${publisherId}`
        dialogOpen.value = true
      }
    } else {
      enroll.error = t('errors.actionFailed')
    }
  } finally {
    enroll.busy = false
  }
}

async function onGranted(grantId: string) {
  dialogOpen.value = false
  const publisherId = enroll.publisherId
  if (!publisherId) return
  enroll.grantId = grantId
  await fetchChallenge(publisherId, grantId)
}

async function submitKey(publisherId: string) {
  if (!enroll.grantId || !enroll.challenge || enroll.busy) return
  enroll.busy = true
  enroll.error = ''
  try {
    await api.requestPublisherKey(
      publisherId,
      {
        challengeId: enroll.challenge.challengeId,
        keyId: enroll.keyId.trim(),
        publicKey: enroll.publicKey.trim(),
        signature: enroll.signature.trim(),
      },
      enroll.grantId,
    )
    ui.toast(t('portal.enrolled'), 'ok')
    enroll.publisherId = null
    enroll.grantId = null
    enroll.challenge = null
  } catch (e) {
    if (e instanceof ApiError && (e.status === 401 || e.status === 403)) {
      enroll.error = stepUpFailed(e.status, e.code)
    } else {
      enroll.error = t('errors.actionFailed')
    }
    enroll.busy = false
  }
}

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

// Google round-trip: the callback redirects here with the one-time grant (or
// an opaque error) in the query. The grant is consumed into memory and the
// query is stripped immediately.
const resumed = ref(false)
async function resumeFromQuery() {
  if (resumed.value) return
  const pending = route.query['stepUp'] === 'enroll' ? route.query['publisher'] : undefined
  if (typeof pending !== 'string' || !publishers.value.some((p) => p.id === pending && canEnroll(p))) return
  resumed.value = true
  const outcome = parseStepUpQuery(route.query as Record<string, unknown>)
  const clean = { ...route.query }
  delete clean['stepUp']
  delete clean['publisher']
  delete clean['stepUpGrant']
  delete clean['stepUpError']
  await router.replace({ path: route.path, query: clean })
  if (outcome && 'grantId' in outcome) {
    enroll.publisherId = pending
    dialogReturnTo.value = `/app/portal?stepUp=enroll&publisher=${pending}`
    await onGranted(outcome.grantId)
  } else if (outcome && 'error' in outcome) {
    ui.toast(t('stepUp.failed'), 'err')
  }
}

onMounted(() => {
  void resumeFromQuery()
})
watch(publishers, () => {
  void resumeFromQuery()
})
</script>
