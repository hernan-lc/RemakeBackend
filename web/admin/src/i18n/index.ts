import { en } from './en'
import { es } from './es'

export type Locale = 'en' | 'es'
export type Dict = typeof en

const catalogs: Record<Locale, Dict> = { en, es }

export function translate(locale: Locale, key: string): string {
  const parts = key.split('.')
  let node: unknown = catalogs[locale] ?? en
  for (const part of parts) {
    if (typeof node !== 'object' || node === null) return key
    node = (node as Record<string, unknown>)[part]
  }
  return typeof node === 'string' ? node : key
}
