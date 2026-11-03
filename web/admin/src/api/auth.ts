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
