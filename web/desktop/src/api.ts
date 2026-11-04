export interface AccountInfo {
  email?: string;
}

export class ApiError extends Error {
  readonly status: number;
  constructor(path: string, method: string, status: number) {
    super(`${method} ${path} -> ${status}`);
    this.status = status;
  }
}

async function json<T>(path: string, init?: RequestInit): Promise<T> {
  const response = await fetch(path, {
    credentials: "same-origin",
    headers: { "Content-Type": "application/json", ...(init?.headers ?? {}) },
    ...init,
  });
  if (!response.ok) throw new ApiError(path, init?.method ?? "GET", response.status);
  return (await response.json()) as T;
}

export function fetchMe(signal?: AbortSignal): Promise<AccountInfo> {
  return json<AccountInfo>("/api/v1/me", { cache: "no-store", signal });
}

export function fetchProviders(): Promise<{ google: boolean }> {
  return json<{ google: boolean }>("/api/v1/auth/providers", { cache: "no-store" });
}

export interface HandoffResponse {
  callbackUrl: string;
}

export function postHandoff(params: {
  redirectUri: string;
  codeChallenge: string;
  state: string;
}): Promise<HandoffResponse> {
  return json<HandoffResponse>("/api/v1/desktop/handoff", {
    method: "POST",
    body: JSON.stringify(params),
  });
}

export interface SignInCredentials {
  email?: string;
  username?: string;
  password: string;
}

export function signIn(credentials: SignInCredentials): Promise<unknown> {
  const email = credentials.email !== undefined;
  return json(email ? "/api/auth/sign-in/email" : "/api/auth/sign-in/username", {
    method: "POST",
    body: JSON.stringify(credentials),
  });
}

export function signUp(input: { email: string; name: string; password: string }): Promise<unknown> {
  return json("/api/auth/sign-up/email", {
    method: "POST",
    body: JSON.stringify(input),
  });
}
