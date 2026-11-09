import { createRouter, createWebHistory, type RouteRecordRaw } from 'vue-router'

const routes: RouteRecordRaw[] = [
  { path: '/', redirect: '/app' },
  { path: '/login', name: 'login', component: () => import('./views/Login.vue') },
  {
    path: '/auth/callback',
    name: 'callback',
    component: () => import('./views/AuthCallback.vue'),
  },
  {
    path: '/app',
    component: () => import('./views/Shell.vue'),
    meta: { requiresAuth: true },
    children: [
      { path: '', name: 'dashboard', component: () => import('./views/Dashboard.vue') },
      { path: 'publishers', name: 'publishers', component: () => import('./views/Publishers.vue') },
      { path: 'moderation', name: 'moderation', component: () => import('./views/Moderation.vue') },
      { path: 'moderation/documents', name: 'document-moderation', component: () => import('./views/DocumentModeration.vue') },
      { path: 'audit', name: 'audit', component: () => import('./views/Audit.vue') },
      { path: 'portal', name: 'portal', component: () => import('./views/Portal.vue') },
      { path: 'account', name: 'account', component: () => import('./views/Account.vue') },
    ],
  },
  { path: '/403', name: 'forbidden', component: () => import('./views/Forbidden.vue') },
  { path: '/logout', name: 'logout', component: () => import('./views/Logout.vue') },
  { path: '/:pathMatch(.*)*', redirect: '/app' },
]

const router = createRouter({ history: createWebHistory(), routes })

// UX-only guard: unauthenticated visits bounce to /login, but every API
// call is authorized server-side regardless of this client routing.
router.beforeEach((to) => {
  if (!to.meta.requiresAuth) return true
  // The Shell loads the session and redirects to /login on 401 itself so
  // the query cache (not a route check) is the source of truth.
  return true
})

export default router
