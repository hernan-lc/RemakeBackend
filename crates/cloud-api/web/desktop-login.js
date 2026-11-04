(() => {
  "use strict";
  const storageKey = "tiktools.desktop.handoff.v1";
  const message = document.getElementById("message");
  const account = document.getElementById("account");
  const signIn = document.getElementById("sign-in");
  const proceed = document.getElementById("continue");
  const cancel = document.getElementById("cancel");
  const passwordForm = document.getElementById("password-form");
  const identifier = document.getElementById("identifier");
  const password = document.getElementById("password");
  const fail = (text) => { message.textContent = text; signIn.hidden = true; passwordForm.hidden = true; proceed.hidden = true; };
  const showSignedOut = async () => {
    message.textContent = "Sign in to continue to TikTools.";
    let google = false;
    try {
      const providers = await fetch("/api/v1/auth/providers", { cache: "no-store" });
      if (providers.ok) google = (await providers.json()).google === true;
    } catch (_) { /* Google stays hidden; password sign-in still works. */ }
    signIn.hidden = !google;
    passwordForm.hidden = false;
  };
  const valid = (data) => {
    if (!data || !/^[A-Za-z0-9_-]{43}$/.test(data.codeChallenge) || !/^[A-Za-z0-9._~-]{16,256}$/.test(data.state)) return false;
    try {
      const url = new URL(data.redirectUri);
      return url.protocol === "http:" && (url.hostname === "127.0.0.1" || url.hostname === "[::1]") && url.port !== "" && url.pathname !== "/" && !url.username && !url.password && !url.search && !url.hash;
    } catch (_) { return false; }
  };
  let pending;
  let cancelled = false;
  try {
    const query = new URLSearchParams(location.search);
    if (location.pathname === "/desktop/login" && location.search) {
      const fields = ["redirectUri", "codeChallenge", "state"];
      if (fields.some((key) => query.getAll(key).length !== 1) || [...query.keys()].some((key) => !fields.includes(key))) throw new Error();
      pending = Object.fromEntries(fields.map((key) => [key, query.get(key)]));
      if (!valid(pending)) throw new Error();
      sessionStorage.setItem(storageKey, JSON.stringify(pending));
      history.replaceState(null, "", "/desktop/login");
    } else {
      pending = JSON.parse(sessionStorage.getItem(storageKey) || "null");
    }
    if (!valid(pending)) throw new Error();
  } catch (_) {
    try { sessionStorage.removeItem(storageKey); } catch (_) {}
    fail("Open TikTools and start sign-in again.");
    return;
  }
  cancel.hidden = false;
  cancel.addEventListener("click", () => {
    cancelled = true;
    sessionStorage.removeItem(storageKey);
    cancel.hidden = true;
    fail("Sign-in cancelled. You can close this window.");
  });
  signIn.addEventListener("click", () => { location.assign("/api/auth/sign-in/social/google"); });
  passwordForm.addEventListener("submit", async (event) => {
    event.preventDefault();
    const id = identifier.value.trim();
    if (!id || !password.value) return;
    message.textContent = "Signing in…";
    const email = id.includes("@");
    try {
      const response = await fetch(email ? "/api/auth/sign-in/email" : "/api/auth/sign-in/username", {
        method: "POST",
        credentials: "same-origin",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify(email ? { email: id, password: password.value } : { username: id, password: password.value }),
      });
      if (!response.ok) throw new Error();
      password.value = "";
      const me = await fetch("/api/v1/me", { credentials: "same-origin", cache: "no-store" });
      if (!me.ok) throw new Error();
      const user = await me.json();
      account.textContent = typeof user.email === "string" ? user.email : "Signed in";
      message.textContent = "Continue to TikTools with this account?";
      signIn.hidden = true;
      passwordForm.hidden = true;
      proceed.hidden = false;
    } catch (_) {
      password.value = "";
      message.textContent = "Sign-in failed. Check the identifier and password, then try again.";
    }
  });
  proceed.addEventListener("click", async () => {
    if (cancelled) return;
    proceed.disabled = true;
    try {
      const response = await fetch("/api/v1/desktop/handoff", { method: "POST", credentials: "same-origin", headers: { "Content-Type": "application/json" }, body: JSON.stringify(pending) });
      if (!response.ok) throw new Error();
      const data = await response.json();
      if (cancelled) return;
      const callback = new URL(data.callbackUrl);
      const expected = new URL(pending.redirectUri);
      if (callback.origin !== expected.origin || callback.pathname !== expected.pathname || callback.searchParams.getAll("state").length !== 1 || callback.searchParams.get("state") !== pending.state || callback.searchParams.getAll("code").length !== 1 || !callback.searchParams.get("code") || callback.searchParams.get("code").length > 128 || [...callback.searchParams.keys()].some((key) => key !== "state" && key !== "code") || callback.username || callback.password || callback.hash) throw new Error();
      sessionStorage.removeItem(storageKey);
      location.assign(callback.href);
    } catch (_) { if (!cancelled) { proceed.disabled = false; fail("Could not complete sign-in. Start again in TikTools."); } }
  });
  fetch("/api/v1/me", { credentials: "same-origin", cache: "no-store" }).then(async (response) => {
    if (cancelled) return;
    if (response.status === 401) { await showSignedOut(); return; }
    if (!response.ok) throw new Error();
    const user = await response.json();
    if (cancelled) return;
    account.textContent = typeof user.email === "string" ? user.email : "Signed in";
    message.textContent = "Continue to TikTools with this account?";
    proceed.hidden = false;
  }).catch(() => { if (!cancelled) fail("Could not check sign-in. Try again."); });
})();
