(() => {
  "use strict";
  const STRINGS = {
    en: {
      title: "TikTools Cloud sign in",
      checking: "Checking sign-in…",
      signInToContinue: "Sign in to continue to TikTools.",
      identifier: "Username or email",
      name: "Display name",
      password: "Password",
      signIn: "Sign in",
      createAccount: "Create account",
      signInGoogle: "Sign in with Google",
      continue: "Continue to TikTools",
      cancel: "Cancel",
      cancelled: "Sign-in cancelled. You can close this window.",
      badHandoff: "Open TikTools and start sign-in again.",
      checkFailed: "Could not check sign-in. Try again.",
      signingIn: "Signing in…",
      signInFailed: "Sign-in failed. Check the identifier and password, then try again.",
      registering: "Creating account…",
      registerFailed: "Could not create the account. The email may already be registered.",
      needRegisterFields: "Enter a display name, an email address and a password.",
      continueWith: "Continue to TikTools with this account?",
      signedIn: "Signed in",
      handoffFailed: "Could not complete sign-in. Start again in TikTools.",
    },
    es: {
      title: "TikTools Cloud: iniciar sesión",
      checking: "Comprobando la sesión…",
      signInToContinue: "Inicia sesión para continuar a TikTools.",
      identifier: "Usuario o correo",
      name: "Nombre visible",
      password: "Contraseña",
      signIn: "Iniciar sesión",
      createAccount: "Crear cuenta",
      signInGoogle: "Iniciar sesión con Google",
      continue: "Continuar a TikTools",
      cancel: "Cancelar",
      cancelled: "Sesión cancelada. Puedes cerrar esta ventana.",
      badHandoff: "Abre TikTools e inicia sesión de nuevo.",
      checkFailed: "No se pudo comprobar la sesión. Inténtalo de nuevo.",
      signingIn: "Iniciando sesión…",
      signInFailed: "Falló el inicio. Revisa el identificador y la contraseña.",
      registering: "Creando la cuenta…",
      registerFailed: "No se pudo crear la cuenta. El correo podría estar registrado.",
      needRegisterFields: "Escribe un nombre visible, un correo y una contraseña.",
      continueWith: "¿Continuar a TikTools con esta cuenta?",
      signedIn: "Sesión iniciada",
      handoffFailed: "No se pudo completar. Empieza de nuevo en TikTools.",
    },
  };
  const langKey = "tiktools.desktop.lang.v1";
  let lang = "en";
  try {
    const stored = localStorage.getItem(langKey);
    if (stored === "en" || stored === "es") lang = stored;
    else if ((navigator.language || "").toLowerCase().startsWith("es")) lang = "es";
  } catch (_) { /* private mode: detection only, no persistence. */ }
  const t = (key) => STRINGS[lang][key] || STRINGS.en[key] || key;
  const applyStrings = () => {
    document.documentElement.lang = lang;
    document.title = t("title");
    document.querySelectorAll("[data-i18n]").forEach((el) => { el.textContent = t(el.dataset.i18n); });
    document.getElementById("lang-en").classList.toggle("active", lang === "en");
    document.getElementById("lang-es").classList.toggle("active", lang === "es");
  };
  const setLang = (next) => {
    lang = next;
    try { localStorage.setItem(langKey, next); } catch (_) {}
    applyStrings();
  };

  const storageKey = "tiktools.desktop.handoff.v1";
  const message = document.getElementById("message");
  const account = document.getElementById("account");
  const signIn = document.getElementById("sign-in");
  const proceed = document.getElementById("continue");
  const cancel = document.getElementById("cancel");
  const passwordForm = document.getElementById("password-form");
  const modeTabs = document.getElementById("mode-tabs");
  const tabLogin = document.getElementById("tab-login");
  const tabRegister = document.getElementById("tab-register");
  const identifier = document.getElementById("identifier");
  const nameField = document.getElementById("name-field");
  const nameInput = document.getElementById("name");
  const password = document.getElementById("password");
  const passwordSignIn = document.getElementById("password-sign-in");
  let mode = "login";
  const setMode = (next) => {
    mode = next;
    nameField.hidden = next !== "register";
    tabLogin.classList.toggle("active", next === "login");
    tabRegister.classList.toggle("active", next === "register");
    passwordSignIn.textContent = next === "register" ? t("createAccount") : t("signIn");
  };
  const fail = (text) => { message.textContent = text; signIn.hidden = true; passwordForm.hidden = true; modeTabs.hidden = true; proceed.hidden = true; };
  const showSignedOut = async () => {
    message.textContent = t("signInToContinue");
    let google = false;
    try {
      const providers = await fetch("/api/v1/auth/providers", { cache: "no-store" });
      if (providers.ok) google = (await providers.json()).google === true;
    } catch (_) { /* Google stays hidden; password sign-in still works. */ }
    signIn.hidden = !google;
    modeTabs.hidden = false;
    passwordForm.hidden = false;
  };
  const signedInAs = async () => {
    const me = await fetch("/api/v1/me", { credentials: "same-origin", cache: "no-store" });
    if (!me.ok) throw new Error();
    const user = await me.json();
    account.textContent = typeof user.email === "string" ? user.email : t("signedIn");
    message.textContent = t("continueWith");
    signIn.hidden = true;
    passwordForm.hidden = true;
    modeTabs.hidden = true;
    proceed.hidden = false;
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
  applyStrings();
  setMode("login");
  document.getElementById("lang-en").addEventListener("click", () => setLang("en"));
  document.getElementById("lang-es").addEventListener("click", () => setLang("es"));
  tabLogin.addEventListener("click", () => setMode("login"));
  tabRegister.addEventListener("click", () => setMode("register"));
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
    fail(t("badHandoff"));
    return;
  }
  cancel.hidden = false;
  cancel.addEventListener("click", () => {
    cancelled = true;
    sessionStorage.removeItem(storageKey);
    cancel.hidden = true;
    fail(t("cancelled"));
  });
  signIn.addEventListener("click", () => { location.assign("/api/auth/sign-in/social/google"); });
  passwordForm.addEventListener("submit", async (event) => {
    event.preventDefault();
    if (mode === "register") {
      const displayName = nameInput.value.trim();
      const email = identifier.value.trim();
      if (!displayName || !email.includes("@") || !password.value) { message.textContent = t("needRegisterFields"); return; }
      message.textContent = t("registering");
      try {
        const response = await fetch("/api/auth/sign-up/email", {
          method: "POST",
          credentials: "same-origin",
          headers: { "Content-Type": "application/json" },
          body: JSON.stringify({ email, name: displayName, password: password.value }),
        });
        if (!response.ok) throw new Error();
        password.value = "";
        await signedInAs();
      } catch (_) {
        password.value = "";
        message.textContent = t("registerFailed");
      }
      return;
    }
    const id = identifier.value.trim();
    if (!id || !password.value) return;
    message.textContent = t("signingIn");
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
      await signedInAs();
    } catch (_) {
      password.value = "";
      message.textContent = t("signInFailed");
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
    } catch (_) { if (!cancelled) { proceed.disabled = false; fail(t("handoffFailed")); } }
  });
  fetch("/api/v1/me", { credentials: "same-origin", cache: "no-store" }).then(async (response) => {
    if (cancelled) return;
    if (response.status === 401) { await showSignedOut(); return; }
    if (!response.ok) throw new Error();
    const user = await response.json();
    if (cancelled) return;
    account.textContent = typeof user.email === "string" ? user.email : t("signedIn");
    message.textContent = t("continueWith");
    proceed.hidden = false;
  }).catch(() => { if (!cancelled) fail(t("checkFailed")); });
})();
