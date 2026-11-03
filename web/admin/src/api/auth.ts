// Pure, unit-testable pieces of the auth flow. Components stay thin;
// these functions decide routing outcomes from already-available facts.
export type CallbackOutcome = 'error' | 'check'

export function callbackOutcome(query: Record<string, unknown>): CallbackOutcome {
  const err = query['error']
  if (typeof err === 'string' && err.length > 0) return 'error'
  return 'check'
}

export function safeNextPath(next: unknown): string {
  // Only allow same-origin absolute paths so a forged ?next= can never
  // bounce the admin to an external site.
  if (typeof next === 'string' && next.startsWith('/') && !next.startsWith('//')) return next
  return '/app'
}

export type StepUpQueryResult = { grantId: string } | { error: string }

/** Read the step-up callback outcome from the route query. The grant (or the
 *  opaque error code) arrives only here, is kept in memory, and is stripped
 *  from the URL as soon as it is consumed. */
export function parseStepUpQuery(query: Record<string, unknown>): StepUpQueryResult | null {
  const grant = query['stepUpGrant']
  if (typeof grant === 'string' && grant.length > 0) return { grantId: grant }
  const error = query['stepUpError']
  if (typeof error === 'string' && error.length > 0) return { error }
  return null
}

/** Return path the OAuth flow resumes after the provider round-trip. Only an
 *  in-application path is accepted, mirroring the backend's own check. */
export function stepUpReturnTo(path: string): string | null {
  if (path === '/app' || path.startsWith('/app/') || path.startsWith('/app?')) {
    if (!path.includes('://') && !path.startsWith('//') && !path.includes('#')) return path
  }
  return null
}
