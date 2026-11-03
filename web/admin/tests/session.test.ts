import { describe, expect, it } from 'vitest'
import { callbackOutcome, safeNextPath } from '../src/api/auth'
import { isCloudAdmin, isReviewer } from '../src/api/types'
import { translate } from '../src/i18n'

describe('auth outcomes', () => {
  it('treats any error param as a failed callback', () => {
    expect(callbackOutcome({ error: 'access_denied' })).toBe('error')
    expect(callbackOutcome({})).toBe('check')
    expect(callbackOutcome({ error: '' })).toBe('check')
    expect(callbackOutcome({ code: 'x', state: 'y' })).toBe('check')
  })

  it('keeps post-login redirects same-origin', () => {
    expect(safeNextPath('/app/moderation')).toBe('/app/moderation')
    expect(safeNextPath('https://evil.example.test')).toBe('/app')
    expect(safeNextPath('//evil.example.test/x')).toBe('/app')
    expect(safeNextPath(undefined)).toBe('/app')
    expect(safeNextPath(42)).toBe('/app')
  })

  it('derives admin/reviewer status from roles only', () => {
    expect(isCloudAdmin({ id: 'u', email: 'a@b.c', name: 'n', roles: ['admin'] })).toBe(true)
    expect(isCloudAdmin({ id: 'u', email: 'a@b.c', name: 'n', roles: ['moderator'] })).toBe(false)
    expect(isCloudAdmin(null)).toBe(false)
    expect(isReviewer({ id: 'u', email: 'a@b.c', name: 'n', roles: ['moderator'] })).toBe(true)
    expect(isReviewer({ id: 'u', email: 'a@b.c', name: 'n', roles: [] })).toBe(false)
  })

  it('falls back to the key and to English for missing translations', () => {
    expect(translate('en', 'auth.signIn')).toBe('Sign in')
    expect(translate('es', 'auth.signIn')).toBe('Iniciar sesión')
    expect(translate('es', 'no.such.key')).toBe('no.such.key')
    expect(translate('en', 'auth')).toBe('auth')
  })
})
