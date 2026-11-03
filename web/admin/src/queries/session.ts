import { useQuery, useQueryClient } from '@tanstack/vue-query'
import { computed } from 'vue'
import { ApiError, api } from '../api/client'
import type { Me } from '../api/types'

export const ME_KEY = ['me']

// Session is server state: loaded on page refresh, cached by the query
// client, invalidated on sign-out. Route guards read this; they are UX
// only — the API authorizes every call independently.
export function useSession() {
  const query = useQuery<Me, ApiError>({
    queryKey: ME_KEY,
    queryFn: api.me,
    retry: (count, error) => error.status >= 500 && count < 1,
    staleTime: 60_000,
  })
  return {
    ...query,
    isSignedIn: computed(() => query.data.value != null),
    isExpired: computed(() => query.error.value?.status === 401),
    isForbidden: computed(() => query.error.value?.status === 403),
  }
}

export function useInvalidateSession() {
  const client = useQueryClient()
  return () => client.invalidateQueries({ queryKey: ME_KEY })
}
