<template>
  <AppCard :label="t('releases.submitTitle')">
    <div class="mb-3"><AppSectionHeader :title="t('releases.submitTitle')" icon="package" /></div>
    <p v-if="eligiblePublishers.length === 0" class="text-sm text-slate-600 dark:text-slate-400">{{ t('releases.selectPublisherHint') }}</p>
    <div v-else ref="wizardBody" class="flex flex-col gap-6">
      <ol class="grid grid-cols-3 gap-2 border-b border-slate-200 pb-4 dark:border-slate-700" :aria-label="copy.steps">
        <li v-for="(label, index) in stageLabels" :key="label" :aria-current="stage === index + 1 ? 'step' : undefined" class="text-xs leading-5" :class="stage === index + 1 ? 'font-semibold text-indigo-700 dark:text-indigo-300' : 'text-slate-600 dark:text-slate-300'">
          <span class="mb-1 block font-mono">0{{ index + 1 }}</span>{{ label }}
        </li>
      </ol>
      <fieldset :disabled="working || validating" class="min-w-0 space-y-6">
      <section data-stage="1" v-show="stage === 1" tabindex="-1" class="space-y-4 focus:outline-none">
        <AppSectionHeader :title="t('releases.sourceDetails')" />
      <AppSelect v-model="publisherId" :label="t('releases.selectPublisher')">
        <option value="" disabled>{{ t('releases.selectPublisher') }}</option>
        <option v-for="p in eligiblePublishers" :key="p.id" :value="p.id">{{ p.namespace }}</option>
      </AppSelect>
      <p v-if="publisherId" class="text-xs text-slate-600 dark:text-slate-300">{{ t('releases.selectPublisherHint') }}</p>

      <StateBlock v-if="publisherId" :loading="keysPending" :error="!!keysError" :retry="true" @retry="refetchKeys">
        <AppSelect v-model="keyId" :label="t('releases.selectKey')">
          <option value="" disabled>{{ t('releases.selectKey') }}</option>
          <option v-for="k in approvedKeys" :key="k.id" :value="k.id">{{ k.keyId }}</option>
        </AppSelect>
      </StateBlock>
      <p v-if="publisherId && !keysPending" class="text-xs text-slate-600 dark:text-slate-300">{{ t('releases.selectKeyHint') }}</p>

      <label class="flex min-w-0 flex-col gap-2 rounded-xl border border-dashed border-slate-300 bg-slate-50 p-4 text-sm dark:border-slate-700 dark:bg-slate-800">
        <span class="font-medium text-slate-700 dark:text-slate-300">{{ t('releases.selectFile') }}</span>
        <input
          :key="fileInputKey"
          type="file"
          accept=".plugin,application/octet-stream"
          class="block w-full max-w-full text-xs text-slate-700 file:mr-3 file:rounded-xl file:border file:border-indigo-200 file:bg-white file:px-3 file:py-2 file:text-xs file:font-semibold file:text-indigo-700 hover:file:bg-indigo-50 dark:text-slate-300 dark:file:border-slate-700 dark:file:bg-slate-800 dark:file:text-indigo-300 dark:hover:file:bg-slate-700"
          @change="onFile"
        />
        <span class="text-xs text-slate-600 dark:text-slate-300">{{ t('releases.fileHint') }}</span>
      </label>
      <dl v-if="fileInfo" class="grid grid-cols-[auto_1fr] gap-x-3 gap-y-2 rounded-xl bg-slate-50 p-4 font-mono text-xs dark:bg-slate-800">
        <dt class="font-sans text-slate-600 dark:text-slate-300">{{ t('releases.checksum') }}</dt>
        <dd class="break-all">{{ fileInfo.sha256 }}</dd>
        <dt class="font-sans text-slate-600 dark:text-slate-300">{{ t('releases.size') }}</dt>
        <dd class="tabular-nums">{{ fileInfo.size }}</dd>
      </dl>

      </section>
      <section data-stage="2" v-show="stage === 2" tabindex="-1" class="space-y-4 focus:outline-none">
        <AppSectionHeader :title="t('releases.packageMetadata')" />
        <AppInput v-model="pluginId" :label="t('releases.pluginId')" required :maxlength="256" />
      <p v-if="publisherId" class="text-xs text-slate-600 dark:text-slate-300">{{ t(namespaceOf(publisherId) === 'tiktools' ? 'releases.officialPluginIdHint' : 'releases.pluginIdHint', { namespace: namespaceOf(publisherId) }) }}</p>
      <div class="grid grid-cols-1 gap-3 sm:grid-cols-2">
        <AppInput v-model="version" :label="t('releases.version')" required :maxlength="128" />
        <AppInput v-model="target" :label="t('releases.target')" required :maxlength="128" />
      </div>
      <p class="text-xs text-slate-600 dark:text-slate-300">{{ t('releases.versionHint') }} {{ t('releases.targetHint') }}</p>

      </section>
      <section data-stage="3" v-if="stage === 3" tabindex="-1" class="space-y-4 focus:outline-none">
        <AppSectionHeader :title="copy.review" />
        <dl class="grid grid-cols-[auto_minmax(0,1fr)] gap-x-4 gap-y-3 text-sm">
          <dt class="text-slate-600 dark:text-slate-300">{{ t('releases.selectPublisher') }}</dt><dd class="break-all font-medium">{{ namespaceOf(publisherId) }}</dd>
          <dt class="text-slate-600 dark:text-slate-300">{{ t('releases.selectKey') }}</dt><dd class="break-all font-mono text-xs">{{ approvedKeys.find(k => k.id === keyId)?.keyId }}</dd>
          <dt class="text-slate-600 dark:text-slate-300">{{ t('releases.selectFile') }}</dt><dd class="break-all">{{ file?.name }}</dd>
          <dt class="text-slate-600 dark:text-slate-300">{{ t('releases.pluginId') }}</dt><dd class="break-all font-mono text-xs">{{ pluginId }}</dd>
          <dt class="text-slate-600 dark:text-slate-300">{{ t('releases.version') }}</dt><dd>{{ version }}</dd>
          <dt class="text-slate-600 dark:text-slate-300">{{ t('releases.target') }}</dt><dd class="break-all">{{ target }}</dd>
          <dt class="text-slate-600 dark:text-slate-300">{{ t('releases.checksum') }}</dt><dd class="break-all font-mono text-xs">{{ fileInfo?.sha256 }}</dd>
        </dl>
        <p class="text-xs leading-5 text-slate-600 dark:text-slate-300">{{ t('releases.fileHint') }}</p>
      </section>
      </fieldset>
      <div class="space-y-4 border-t border-slate-200 pt-5 dark:border-slate-700">
        <AppSectionHeader v-if="stage === 3" :title="t('releases.uploadProgress')" />
      <p v-if="error" role="alert" class="flex items-start gap-2 text-sm text-red-700 dark:text-red-300">
        <AppIcon name="alert" :size="16" class="mt-0.5 shrink-0" aria-hidden="true" />
        {{ error }}
      </p>
      <p v-if="notice" role="status" class="flex items-start gap-2 text-sm text-slate-700 dark:text-slate-300">
        <AppIcon name="checkCircle" :size="16" class="mt-0.5 shrink-0" aria-hidden="true" />
        {{ notice }}
      </p>

      <div class="flex flex-wrap items-center justify-between gap-3">
        <AppButton v-if="stage > 1 && phase !== 'done'" size="sm" variant="secondary" :disabled="working || validating" @click="stage--; error = ''">{{ copy.back }}</AppButton>
        <AppButton v-if="stage < 3" size="sm" variant="primary" :loading="validating" :disabled="validating || (stage === 1 && (!publisherId || !keyId || !file))" @click="advance">{{ copy.next }}</AppButton>
        <AppButton v-if="stage === 3" size="sm" variant="primary" icon="package" class="w-full sm:w-auto" :loading="phase !== 'idle' && phase !== 'done'" :disabled="!canStart || phase === 'done' || working || validating" @click="run">
          {{ phase === 'done' ? t('releases.submitted') : t('releases.submit') }}
        </AppButton>
        <AppButton v-if="phase === 'done'" size="sm" variant="secondary" @click="reset">
          {{ t('states.retry') }}
        </AppButton>
      </div>
      <ol v-if="phase !== 'idle'" class="flex flex-col gap-1 text-xs text-slate-600 dark:text-slate-400">
        <li v-for="s in steps" :key="s.key" class="flex items-center gap-2">
          <AppIcon :name="s.done ? 'checkCircle' : s.active ? 'refresh' : 'clock'" :size="14" aria-hidden="true" />
          {{ s.label }}
        </li>
      </ol>
      </div>
    </div>
  </AppCard>
</template>

<script setup lang="ts">
import { computed, nextTick, ref, watch } from 'vue'
import { useI18n } from 'vue-i18n'
import { useQuery } from '@tanstack/vue-query'
import { ApiError, api } from '../../api/client'
import { publisherAllowsPlugin } from '../../api/publisher-scope'
import { canPublish } from '../../api/types'
import type { MyPublisher } from '../../api/types'
import StateBlock from '../StateBlock.vue'
import AppIcon from '../icons/AppIcon.vue'
import AppButton from '../ui/AppButton.vue'
import AppCard from '../ui/AppCard.vue'
import AppInput from '../ui/AppInput.vue'
import AppSelect from '../ui/AppSelect.vue'
import AppSectionHeader from '../ui/AppSectionHeader.vue'

const props = defineProps<{ publishers: MyPublisher[] }>()
const emit = defineEmits<{ submitted: [] }>()

const { t, locale } = useI18n()
const stage = ref(1)
const wizardBody = ref<HTMLElement | null>(null)
watch(stage, async value => {
  await nextTick()
  wizardBody.value?.querySelector<HTMLElement>(`[data-stage="${value}"]`)?.focus()
})
const validating = ref(false)
const fileInputKey = ref(0)
const copy = computed(() => locale.value === 'es'
  ? { steps: 'Pasos de publicación', package: 'Paquete firmado', metadata: 'Metadatos', review: 'Revisar y enviar', back: 'Atrás', next: 'Continuar' }
  : { steps: 'Publication steps', package: 'Signed package', metadata: 'Metadata', review: 'Review and submit', back: 'Back', next: 'Continue' })
const stageLabels = computed(() => [copy.value.package, copy.value.metadata, copy.value.review])

const MAX_SIZE = 128 * 1024 * 1024
const eligiblePublishers = computed(() => props.publishers.filter((p) => p.status === 'approved' && canPublish(p.role)))

const publisherId = ref('')
const keyId = ref('')
const file = ref<File | null>(null)
const fileInfo = ref<{ sha256: string; size: number } | null>(null)
const pluginId = ref('')
const version = ref('')
const target = ref('any')
const phase = ref<'idle' | 'reserving' | 'uploading' | 'completing' | 'submitting' | 'done'>('idle')
const working = computed(() => phase.value !== 'idle' && phase.value !== 'done')
const error = ref('')
const notice = ref('')

function namespaceOf(id: string): string {
  return props.publishers.find((p) => p.id === id)?.namespace ?? ''
}

const { data: keys, isPending: keysPending, error: keysError, refetch: refetchKeys } = useQuery({
  queryKey: ['portal', 'wizard-keys', publisherId],
  queryFn: () => api.publisherKeys(publisherId.value),
  enabled: computed(() => publisherId.value !== ''),
})
const approvedKeys = computed(() => (keys.value ?? []).filter((k) => k.status === 'approved'))
watch(publisherId, () => {
  keyId.value = ''
  if (!working.value) stage.value = 1
})

const canStart = computed(
  () => eligiblePublishers.value.some(p => p.id === publisherId.value) && approvedKeys.value.some(k => k.id === keyId.value) && file.value !== null && pluginId.value.trim() !== '' && version.value.trim() !== '' && target.value.trim() !== '',
)

const steps = computed(() => {
  const order = ['reserving', 'uploading', 'completing', 'submitting'] as const
  const labels: Record<string, string> = {
    reserving: t('releases.reserve'),
    uploading: t('releases.upload'),
    completing: t('releases.complete'),
    submitting: t('releases.submit'),
  }
  const activeIndex = phase.value === 'idle' || phase.value === 'done' ? order.length : order.indexOf(phase.value)
  return order.map((key, i) => ({ key, label: labels[key]!, done: i < activeIndex || phase.value === 'done', active: phase.value === key }))
})

function onFile(event: Event) {
  const input = event.target as HTMLInputElement
  file.value = input.files?.[0] ?? null
  fileInfo.value = null
}

function validVersion(v: string): boolean {
  const m = /^(\d+)\.(\d+)\.(\d+)(?:-([0-9A-Za-z.-]+))?(?:\+([0-9A-Za-z.-]+))?$/.exec(v)
  if (!m) return false
  for (const part of [m[1]!, m[2]!, m[3]!]) {
    if (part.length > 1 && part.startsWith('0')) return false
  }
  return true
}

function newIdempotencyKey(): string {
  if (typeof crypto !== 'undefined' && 'randomUUID' in crypto) return crypto.randomUUID()
  return `portal-${Date.now()}-${Math.floor(Math.random() * 1e9)}`
}

async function hashFile(f: File): Promise<string> {
  const subtle = typeof crypto !== 'undefined' ? crypto.subtle : undefined
  if (!subtle) throw new Error('no-subtle')
  const digest = await subtle.digest('SHA-256', await f.arrayBuffer())
  return [...new Uint8Array(digest)].map((b) => b.toString(16).padStart(2, '0')).join('')
}

async function validateLocal(f: File): Promise<string> {
  if (f.size < 1 || f.size > MAX_SIZE) throw new Error('size')
  const head = new Uint8Array(await f.slice(0, 4).arrayBuffer())
  if (!(head[0] === 0x50 && head[1] === 0x4b && (head[2] === 0x03 || head[2] === 0x05 || head[2] === 0x07) && (head[3] === 0x04 || head[3] === 0x06 || head[3] === 0x08))) {
    throw new Error('magic')
  }
  return hashFile(f)
}

function failExpired(status: number): boolean {
  return status === 403 || status === 410 || status === 404
}

async function advance() {
  if (validating.value || working.value) return
  error.value = ''
  if (stage.value === 1) {
    if (!file.value || !publisherId.value || !approvedKeys.value.some(k => k.id === keyId.value)) return
    validating.value = true
    try {
      fileInfo.value = { sha256: await validateLocal(file.value), size: file.value.size }
      stage.value = 2
    } catch (e) {
      error.value = e instanceof Error && e.message === 'size' ? t('releases.tooLarge') : e instanceof Error && e.message === 'magic' ? t('releases.badMagic') : t('releases.noSha')
    } finally { validating.value = false }
  } else {
    const namespace = namespaceOf(publisherId.value)
    if (!publisherAllowsPlugin(namespace, pluginId.value.trim())) {
      error.value = t(namespace === 'tiktools' ? 'releases.officialPluginIdMismatch' : 'releases.pluginIdMismatch', { namespace }); return
    }
    if (!validVersion(version.value.trim())) { error.value = t('releases.versionInvalid'); return }
    if (!target.value.trim()) return
    stage.value = 3
  }
}

async function run() {
  if (working.value || validating.value) return
  if (!canStart.value || phase.value === 'done' || !file.value) {
    error.value = t('releases.invalidFile')
    return
  }
  error.value = ''
  notice.value = ''
  const namespace = namespaceOf(publisherId.value)
  if (!publisherAllowsPlugin(namespace, pluginId.value.trim())) {
    error.value = t(namespace === 'tiktools' ? 'releases.officialPluginIdMismatch' : 'releases.pluginIdMismatch', { namespace })
    return
  }
  if (!validVersion(version.value.trim())) {
    error.value = t('releases.versionInvalid')
    return
  }
  const currentFile = file.value
  const submission = { publisherId: publisherId.value, publisherKeyId: keyId.value, pluginId: pluginId.value.trim(), version: version.value.trim(), target: target.value.trim() }
  validating.value = true
  // 1-2: file validation is local; the server revalidates everything.
  let sha256: string
  try {
    sha256 = await validateLocal(currentFile)
  } catch (e) {
    error.value =
      e instanceof Error && e.message === 'size'
        ? t('releases.tooLarge')
        : e instanceof Error && e.message === 'magic'
          ? t('releases.badMagic')
          : t('releases.noSha')
    return
  } finally { validating.value = false }
  fileInfo.value = { sha256, size: currentFile.size }
  // 3-6: reserve, upload, complete, submit. Each step is retried explicitly by
  // re-running; reservations are idempotent per key and expire in 5 minutes.
  try {
    phase.value = 'reserving'
    const reservation = await api.reserveUpload(submission.publisherId, currentFile.size, sha256, newIdempotencyKey())
    notice.value = t('releases.reserved')
    phase.value = 'uploading'
    const put = await fetch(reservation.url, {
      method: 'PUT',
      headers: reservation.headers,
      body: currentFile,
      credentials: 'omit',
    })
    if (!put.ok) {
      error.value = failExpired(put.status) ? t('releases.expiredUpload') : t('releases.uploadFailed')
      phase.value = 'idle'
      return
    }
    phase.value = 'completing'
    const completed = await api.completeUpload(reservation.id)
    if (completed.status !== 'quarantined') {
      error.value = t('releases.uploadFailed')
      phase.value = 'idle'
      return
    }
    notice.value = t('releases.uploaded')
    phase.value = 'submitting'
    const release = await api.submitRelease({
      uploadId: reservation.id,
      publisherKeyId: submission.publisherKeyId,
      pluginId: submission.pluginId,
      version: submission.version,
      target: submission.target,
    })
    phase.value = 'done'
    notice.value = `${t('releases.submitted')} (${release.id})`
    emit('submitted')
  } catch (e) {
    if (e instanceof ApiError) {
      if (e.status === 401) error.value = t('errors.expired')
      else if (e.status === 403 || e.status === 410) error.value = t('releases.expiredUpload')
      else if (e.status === 404 || e.status === 409) error.value = t('releases.submitFailed')
      else error.value = t('errors.actionFailed')
    } else {
      error.value = t('errors.actionFailed')
    }
    phase.value = 'idle'
  }
}

function reset() {
  stage.value = 1
  fileInputKey.value++
  phase.value = 'idle'
  error.value = ''
  notice.value = ''
  file.value = null
  fileInfo.value = null
  pluginId.value = ''
  version.value = ''
  target.value = 'any'
  keyId.value = ''
}

defineExpose({ phase: computed(() => phase.value), error: computed(() => error.value) })
</script>
