// Typed same-origin API client. Cookie sessions only: credentials are
// included on every request and no token is ever persisted anywhere
// (no localStorage, no sessionStorage).
import type {
  AuditPage,
  Inspection,
  Me,
  ModerationQueue,
  MyPublisher,
  Overview,
  PublisherMember,
  PublisherReviewPage,
} from './types'

export class ApiError extends Error {
  status: number
  constructor(status: number, message: string) {
    super(message)
    this.status = status
  }
}

async function request<T>(path: string, init?: RequestInit): Promise<T> {
  const response = await fetch(path, {
    credentials: 'include',
    ...init,
    headers: { 'Content-Type': 'application/json', ...(init?.headers ?? {}) },
  })
  if (!response.ok) {
    throw new ApiError(response.status, `${init?.method ?? 'GET'} ${path} -> ${response.status}`)
  }
  if (response.status === 204) return undefined as T
  return (await response.json()) as T
}

export const api = {
  me: () => request<Me>('/api/v1/me'),
  signOut: () =>
    request<unknown>('/api/auth/sign-out', { method: 'POST', body: JSON.stringify({}) }),
  signInEmail: (email: string, password: string) =>
    request<unknown>('/api/auth/sign-in/email', {
      method: 'POST',
      body: JSON.stringify({ email, password }),
    }),
  overview: () => request<Overview>('/api/v1/admin/overview'),
  adminPublishers: (params: { status?: string; cursor?: string; limit?: number }) => {
    const q = new URLSearchParams()
    if (params.status) q.set('status', params.status)
    if (params.cursor) q.set('cursor', params.cursor)
    if (params.limit) q.set('limit', String(params.limit))
    const s = q.toString()
    return request<PublisherReviewPage>(`/api/v1/admin/publishers${s ? `?${s}` : ''}`)
  },
  approvePublisher: (id: string) =>
    request<unknown>(`/api/v1/admin/publishers/${id}/approve`, { method: 'POST' }),
  audit: (params: { cursor?: string; limit?: number }) => {
    const q = new URLSearchParams()
    if (params.cursor) q.set('cursor', params.cursor)
    if (params.limit) q.set('limit', String(params.limit))
    const s = q.toString()
    return request<AuditPage>(`/api/v1/admin/audit${s ? `?${s}` : ''}`)
  },
  moderationQueue: (after?: string) =>
    request<ModerationQueue>(
      `/api/v1/moderation/releases${after ? `?after=${encodeURIComponent(after)}` : ''}`,
    ),
  inspectRelease: (id: string) =>
    request<Inspection>(`/api/v1/moderation/releases/${id}/inspection`, { method: 'POST' }),
  decideRelease: (id: string, decision: 'approve' | 'reject', reasonCode: string) =>
    request<unknown>(`/api/v1/moderation/releases/${id}/decision`, {
      method: 'POST',
      body: JSON.stringify({ decision, reasonCode }),
    }),
  yankRelease: (id: string, reason: string) =>
    request<unknown>(`/api/v1/moderation/releases/${id}/yank`, {
      method: 'POST',
      body: JSON.stringify({ reason }),
    }),
  recordAdvisory: (id: string, severity: string, message: string, url?: string) =>
    request<unknown>(`/api/v1/moderation/releases/${id}/advisory`, {
      method: 'POST',
      body: JSON.stringify({ severity, message, url: url || null }),
    }),
  myPublishers: () => request<MyPublisher[]>('/api/v1/publishers'),
  publisherMembers: (id: string) => request<PublisherMember[]>(`/api/v1/publishers/${id}/members`),
}
