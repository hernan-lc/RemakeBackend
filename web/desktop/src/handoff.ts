export interface HandoffParams {
  redirectUri: string;
  codeChallenge: string;
  state: string;
}

const CODE_CHALLENGE = /^[A-Za-z0-9_-]{43}$/;
const STATE = /^[A-Za-z0-9._~-]{16,256}$/;

export function isValidHandoff(data: unknown): data is HandoffParams {
  if (!data || typeof data !== "object") return false;
  const params = data as Record<string, unknown>;
  if (
    typeof params.codeChallenge !== "string" ||
    typeof params.state !== "string" ||
    typeof params.redirectUri !== "string" ||
    !CODE_CHALLENGE.test(params.codeChallenge) ||
    !STATE.test(params.state)
  ) {
    return false;
  }
  try {
    const url = new URL(params.redirectUri);
    return (
      url.protocol === "http:" &&
      (url.hostname === "127.0.0.1" || url.hostname === "[::1]") &&
      url.port !== "" &&
      url.pathname !== "/" &&
      !url.username &&
      !url.password &&
      !url.search &&
      !url.hash
    );
  } catch {
    return false;
  }
}

export function readHandoffFromLocation(): HandoffParams | null {
  if (location.pathname !== "/desktop/login" || !location.search) return null;
  const query = new URLSearchParams(location.search);
  const fields = ["redirectUri", "codeChallenge", "state"] as const;
  if (
    fields.some((key) => query.getAll(key).length !== 1) ||
    [...query.keys()].some((key) => !(fields as readonly string[]).includes(key))
  ) {
    return null;
  }
  const params: HandoffParams = {
    redirectUri: query.get("redirectUri") ?? "",
    codeChallenge: query.get("codeChallenge") ?? "",
    state: query.get("state") ?? "",
  };
  return isValidHandoff(params) ? params : null;
}

export interface HandoffCallback {
  href: string;
}

export function checkCallbackUrl(
  callbackHref: string,
  pending: HandoffParams,
): HandoffCallback | null {
  let callback: URL;
  let expected: URL;
  try {
    callback = new URL(callbackHref);
    expected = new URL(pending.redirectUri);
  } catch {
    return null;
  }
  const keys = [...callback.searchParams.keys()];
  const ok =
    callback.origin === expected.origin &&
    callback.pathname === expected.pathname &&
    callback.searchParams.getAll("state").length === 1 &&
    callback.searchParams.get("state") === pending.state &&
    callback.searchParams.getAll("code").length === 1 &&
    (callback.searchParams.get("code")?.length ?? 0) > 0 &&
    (callback.searchParams.get("code")?.length ?? 129) <= 128 &&
    keys.every((key) => key === "state" || key === "code") &&
    !callback.username &&
    !callback.password &&
    !callback.hash;
  return ok ? { href: callback.href } : null;
}
