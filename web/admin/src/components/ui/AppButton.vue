<template>
  <component
    :is="to ? RouterLink : href ? 'a' : 'button'"
    :to="to"
    :href="href"
    :type="to || href ? undefined : (type ?? 'button')"
    :disabled="to || href ? undefined : (disabled || loading)"
    :aria-busy="loading || undefined"
    :aria-disabled="disabled || loading || undefined"
    :class="classes"
  >
    <AppIcon v-if="icon && !loading" :name="icon" :size="iconSize" aria-hidden="true" />
    <span v-if="loading" class="inline-block size-4 animate-spin rounded-full border-2 border-current border-t-transparent" aria-hidden="true" />
    <span v-if="$slots.default" :class="compact ? undefined : 'truncate'"><slot /></span>
  </component>
</template>

<script setup lang="ts">
import { computed, useSlots } from 'vue'
import { RouterLink } from 'vue-router'
import AppIcon from '../icons/AppIcon.vue'
import type { IconName } from '../icons/icons'

type Variant = 'primary' | 'secondary' | 'danger' | 'dangerFill' | 'ghost'
type Size = 'sm' | 'md'

const props = withDefaults(
  defineProps<{
    variant?: Variant
    size?: Size
    to?: string | Record<string, unknown>
    href?: string
    type?: 'button' | 'submit' | 'reset'
    disabled?: boolean
    loading?: boolean
    icon?: IconName
    /** Icon-only button: keeps a square hit area, label becomes tooltip/aria. */
    iconOnly?: boolean
    label?: string
  }>(),
  { variant: 'secondary', size: 'md', to: undefined, href: undefined, type: 'button', disabled: false, loading: false, icon: undefined, iconOnly: false, label: undefined },
)

const slots = useSlots()
const compact = computed(() => props.iconOnly || (!slots.default && !!props.icon))

const variantClasses: Record<Variant, string> = {
  primary:
    'bg-slate-900 text-white hover:bg-slate-700 active:bg-slate-800 dark:bg-slate-100 dark:text-slate-900 dark:hover:bg-white dark:active:bg-slate-200',
  secondary:
    'border border-slate-300 text-slate-700 hover:bg-slate-100 active:bg-slate-200 dark:border-slate-600 dark:text-slate-200 dark:hover:bg-slate-800 dark:active:bg-slate-700',
  danger:
    'border border-red-300 text-red-700 hover:bg-red-50 active:bg-red-100 dark:border-red-800 dark:text-red-300 dark:hover:bg-red-950 dark:active:bg-red-900',
  dangerFill:
    'bg-red-700 text-white hover:bg-red-600 active:bg-red-800 dark:bg-red-600 dark:hover:bg-red-500 dark:active:bg-red-700',
  ghost: 'text-slate-600 hover:bg-slate-100 hover:text-slate-900 dark:text-slate-300 dark:hover:bg-slate-800 dark:hover:text-white',
}

const classes = computed(() => [
  'inline-flex max-w-full items-center justify-center gap-1.5 rounded-md font-medium transition-colors',
  'disabled:cursor-not-allowed disabled:opacity-50',
  props.size === 'sm' ? 'px-2.5 py-1 text-sm' : 'px-3.5 py-2 text-sm',
  compact.value ? (props.size === 'sm' ? 'size-8 !px-0' : 'size-10 !px-0') : undefined,
  variantClasses[props.variant],
])

const iconSize = computed(() => (props.size === 'sm' ? 15 : 17))
</script>
