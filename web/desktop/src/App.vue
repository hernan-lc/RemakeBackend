<script setup lang="ts">
import { onMounted, ref, watchEffect } from "vue";
import CollapseTransition from "./components/CollapseTransition.vue";
import LangSelect from "./components/LangSelect.vue";
import PasswordField from "./components/PasswordField.vue";
import TextField from "./components/TextField.vue";
import { ApiError, fetchMe, fetchProviders, postHandoff, signIn, signUp } from "./api";
import { checkCallbackUrl, isValidHandoff, readHandoffFromLocation, type HandoffParams } from "./handoff";
import { STRINGS, detectLanguage, persistLanguage, type Language, type MessageKey } from "./i18n";

const HANDOFF_KEY = "tiktools.desktop.handoff.v1";

const lang = ref<Language>(detectLanguage());
const mode = ref<"login" | "register">("login");
const step = ref<1 | 2>(1);
const identifier = ref("");
const displayName = ref("");
const password = ref("");
const account = ref("");
const message = ref(STRINGS[lang.value].checking);
const googleAvailable = ref(false);
const canContinue = ref(false);
const signedIn = ref(false);
const pending = ref<HandoffParams | null>(null);
const cancelled = ref(false);
const busy = ref(false);

function t(key: MessageKey): string {
  return STRINGS[lang.value][key];
}

function setLang(next: Language): void {
  lang.value = next;
  persistLanguage(next);
}

watchEffect(() => {
  document.documentElement.lang = lang.value;
  document.title = t("title");
});

function loadHandoff(): boolean {
  try {
    const fromLocation = readHandoffFromLocation();
    if (fromLocation) {
      pending.value = fromLocation;
      sessionStorage.setItem(HANDOFF_KEY, JSON.stringify(fromLocation));
      history.replaceState(null, "", "/desktop/login");
    } else {
      const stored: unknown = JSON.parse(sessionStorage.getItem(HANDOFF_KEY) ?? "null");
      pending.value = isValidHandoff(stored) ? stored : null;
    }
    return pending.value !== null;
  } catch {
    try {
      sessionStorage.removeItem(HANDOFF_KEY);
    } catch {
      /* Storage unavailable: the flow cannot continue. */
    }
    return false;
  }
}

function fail(text: string): void {
  message.value = text;
  signedIn.value = false;
  canContinue.value = false;
}

function showStep(next: 1 | 2): void {
  step.value = next;
  if (next === 2) message.value = t("enterPassword");
}

function setMode(next: "login" | "register"): void {
  mode.value = next;
  if (next === "register") step.value = 1;
  else showStep(1);
}

async function showSignedIn(email: string | undefined): Promise<void> {
  account.value = typeof email === "string" ? email : t("signedIn");
  message.value = t("continueWith");
  signedIn.value = true;
}

async function showSignedOut(): Promise<void> {
  // No placeholder line: the form explains itself, and the primary
  // actions carry the context as hover tooltips instead.
  message.value = "";
  try {
    googleAvailable.value = (await fetchProviders()).google === true;
  } catch {
    googleAvailable.value = false;
  }
}

async function login(): Promise<void> {
  const id = identifier.value.trim();
  if (!id || !password.value || busy.value) return;
  busy.value = true;
  message.value = t("signingIn");
  try {
    await signIn(
      id.includes("@")
        ? { email: id, password: password.value }
        : { username: id, password: password.value },
    );
    password.value = "";
    const user = await fetchMe();
    await showSignedIn(user.email);
  } catch {
    password.value = "";
    message.value = t("signInFailed");
  } finally {
    busy.value = false;
  }
}

async function register(): Promise<void> {
  const name = displayName.value.trim();
  const email = identifier.value.trim();
  if (!name || !email.includes("@") || !password.value || busy.value) {
    message.value = t("needRegisterFields");
    return;
  }
  busy.value = true;
  message.value = t("registering");
  try {
    await signUp({ email, name, password: password.value });
    password.value = "";
    const user = await fetchMe();
    await showSignedIn(user.email);
  } catch {
    password.value = "";
    message.value = t("registerFailed");
  } finally {
    busy.value = false;
  }
}

async function submit(): Promise<void> {
  if (mode.value === "register") {
    await register();
    return;
  }
  if (step.value === 1) {
    if (!identifier.value.trim()) return;
    showStep(2);
    return;
  }
  await login();
}

function goBack(): void {
  message.value = "";
  showStep(1);
}

function cancel(): void {
  cancelled.value = true;
  try {
    sessionStorage.removeItem(HANDOFF_KEY);
  } catch {
    /* Already gone. */
  }
  fail(t("cancelled"));
}

function signInWithGoogle(): void {
  location.assign("/api/auth/sign-in/social/google");
}

async function cont(): Promise<void> {
  const handoff = pending.value;
  if (cancelled.value || !handoff || busy.value) return;
  busy.value = true;
  try {
    const { callbackUrl } = await postHandoff(handoff);
    if (cancelled.value) return;
    const checked = checkCallbackUrl(callbackUrl, handoff);
    if (!checked) throw new Error("callback mismatch");
    try {
      sessionStorage.removeItem(HANDOFF_KEY);
    } catch {
      /* Already gone. */
    }
    location.assign(checked.href);
  } catch {
    if (!cancelled.value) {
      busy.value = false;
      fail(t("handoffFailed"));
    }
  }
}

onMounted(async () => {
  if (!loadHandoff()) {
    fail(t("badHandoff"));
    return;
  }
  try {
    const user = await fetchMe();
    account.value = typeof user.email === "string" ? user.email : t("signedIn");
    message.value = t("continueWith");
    canContinue.value = true;
  } catch (error) {
    if (cancelled.value) return;
    if (error instanceof ApiError && error.status === 401) {
      await showSignedOut();
      return;
    }
    fail(t("checkFailed"));
  }
});
</script>

<template>
  <main>
    <div class="topbar">
      <div class="brand">
        <svg class="logo" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
          <path d="M17.5 19a4.5 4.5 0 0 0 .4-8.98 6 6 0 0 0-11.7 1.62A4 4 0 0 0 7 19h10.5z"/>
          <path d="M9.5 15.5l2 2 3.5-4"/>
        </svg>
      </div>
      <LangSelect :lang="lang" @select="setLang" />
    </div>
    <p id="message" role="status">{{ message }}</p>
    <p id="account">{{ account }}</p>

    <Transition name="fade" mode="out-in">
      <div v-if="!signedIn && !canContinue" key="form">
        <div class="tabs" id="mode-tabs">
          <button type="button" :class="{ active: mode === 'login' }" @click="setMode('login')">
            {{ t("signIn") }}
          </button>
          <button type="button" :class="{ active: mode === 'register' }" @click="setMode('register')">
            {{ t("createAccount") }}
          </button>
        </div>

        <form id="password-form" @submit.prevent="submit">
          <CollapseTransition :open="mode === 'register' || step === 1">
            <TextField
              v-model="identifier"
              :label="t('identifier')"
              icon="user"
              autocomplete="username"
            />
          </CollapseTransition>
          <CollapseTransition :open="mode === 'login' && step === 2">
            <p class="who">{{ identifier }}</p>
          </CollapseTransition>

          <CollapseTransition :open="mode === 'register'">
            <TextField
              v-model="displayName"
              :label="t('name')"
              icon="tag"
              autocomplete="nickname"
            />
          </CollapseTransition>

          <CollapseTransition :open="mode === 'register' || step === 2">
            <PasswordField
              v-model="password"
              :label="t('password')"
              autocomplete="current-password"
              :show-label="t('showPassword')"
              :hide-label="t('hidePassword')"
            />
          </CollapseTransition>

          <div class="actions">
            <button
              v-if="mode === 'login' && step === 1"
              type="submit"
              id="password-next"
              :disabled="busy"
              :title="t('signInToContinue')"
            >
              {{ t("next") }}
            </button>
            <button
              v-else
              type="submit"
              id="password-sign-in"
              :disabled="busy"
              :title="mode === 'register' ? t('needRegisterFields') : t('signInToContinue')"
            >
              {{ mode === "register" ? t("createAccount") : t("signIn") }}
            </button>
            <button
              v-if="mode === 'login' && step === 2"
              type="button"
              id="password-back"
              class="ghost"
              @click="goBack"
            >
              {{ t("back") }}
            </button>
          </div>
        </form>

        <div class="actions">
          <button
            v-if="googleAvailable"
            type="button"
            id="sign-in"
            :title="t('signInToContinue')"
            @click="signInWithGoogle"
          >
            <svg class="g" viewBox="0 0 24 24" aria-hidden="true">
              <path fill="#4285F4" d="M23.5 12.3c0-.9-.1-1.5-.3-2.3H12v4.3h6.5c-.1 1.1-.8 2.7-2.4 3.8l-.1.1 3.5 2.7.2.1c2.2-2 3.8-5 3.8-8.7z"/>
              <path fill="#34A853" d="M12 24c3.2 0 5.9-1.1 7.9-2.9l-3.8-2.9c-1 .7-2.4 1.2-4.1 1.2-3.1 0-5.8-2.1-6.8-5l-.1.1-3.6 2.8v.1C3.5 21.4 7.5 24 12 24z"/>
              <path fill="#FBBC05" d="M5.2 14.4c-.2-.7-.4-1.5-.4-2.4s.1-1.7.4-2.4l-.1-.1-3.6-2.8-.1.1C.5 8.6 0 10.2 0 12s.5 3.4 1.4 4.9l3.8-2.5z"/>
              <path fill="#EA4335" d="M12 4.7c1.8 0 3 .8 3.7 1.4l3.3-3.2C17.9 1.1 15.2 0 12 0 7.5 0 3.5 2.6 1.4 6.8l3.8 2.9c1-2.9 3.7-5 6.8-5z"/>
            </svg>
            <span>{{ t("signInGoogle") }}</span>
          </button>
        </div>
      </div>

      <div v-else key="done" class="actions">
        <button type="button" id="continue" :disabled="busy" @click="cont">
          {{ t("continue") }}
        </button>
      </div>
    </Transition>

    <div class="actions">
      <button type="button" id="cancel" class="ghost" @click="cancel">
        {{ t("cancel") }}
      </button>
    </div>
  </main>
</template>
