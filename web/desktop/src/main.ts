import "./style.css";
import {
  LANGUAGES,
  LANGUAGE_LABELS,
  STRINGS,
  detectLanguage,
  persistLanguage,
  type Language,
  type MessageKey,
} from "./i18n";
import { ApiError, fetchMe, fetchProviders, postHandoff, signIn, signUp } from "./api";
import {
  checkCallbackUrl,
  isValidHandoff,
  readHandoffFromLocation,
  type HandoffParams,
} from "./handoff";

const HANDOFF_KEY = "tiktools.desktop.handoff.v1";

function el<T extends HTMLElement>(id: string): T {
  const node = document.getElementById(id);
  if (!node) throw new Error(`missing #${id}`);
  return node as T;
}

class ConsentPage {
  private lang: Language = detectLanguage();
  private pending: HandoffParams | null = null;
  private cancelled = false;
  private mode: "login" | "register" = "login";

  private message = el("message");
  private account = el("account");
  private signIn = el<HTMLButtonElement>("sign-in");
  private proceed = el<HTMLButtonElement>("continue");
  private cancel = el<HTMLButtonElement>("cancel");
  private passwordForm = el<HTMLFormElement>("password-form");
  private modeTabs = el("mode-tabs");
  private tabLogin = el<HTMLButtonElement>("tab-login");
  private tabRegister = el<HTMLButtonElement>("tab-register");
  private identifier = el<HTMLInputElement>("identifier");
  private nameField = el("name-field");
  private nameInput = el<HTMLInputElement>("name");
  private password = el<HTMLInputElement>("password");
  private passwordSignIn = el<HTMLButtonElement>("password-sign-in");

  private t(key: MessageKey): string {
    return STRINGS[this.lang][key];
  }

  private applyStrings(): void {
    document.documentElement.lang = this.lang;
    document.title = this.t("title");
    document.querySelectorAll<HTMLElement>("[data-i18n]").forEach((node) => {
      const key = node.dataset.i18n as MessageKey | undefined;
      if (key && node.childElementCount === 0) node.textContent = this.t(key);
    });
    this.langContainer.querySelectorAll<HTMLButtonElement>("button").forEach((button) => {
      button.classList.toggle("active", button.dataset.lang === this.lang);
    });
  }

  private langContainer = el("lang");

  private renderLanguages(): void {
    this.langContainer.replaceChildren();
    for (const code of LANGUAGES) {
      const button = document.createElement("button");
      button.type = "button";
      button.dataset.lang = code;
      button.textContent = LANGUAGE_LABELS[code];
      button.setAttribute("aria-label", code);
      button.addEventListener("click", () => this.setLang(code));
      this.langContainer.append(button);
    }
  }

  private setLang(next: Language): void {
    this.lang = next;
    persistLanguage(next);
    this.applyStrings();
  }

  private setMode(next: "login" | "register"): void {
    this.mode = next;
    this.nameField.hidden = next !== "register";
    this.tabLogin.classList.toggle("active", next === "login");
    this.tabRegister.classList.toggle("active", next === "register");
    this.passwordSignIn.textContent = this.t(next === "register" ? "createAccount" : "signIn");
  }

  private fail(text: string): void {
    this.message.textContent = text;
    this.signIn.hidden = true;
    this.passwordForm.hidden = true;
    this.modeTabs.hidden = true;
    this.proceed.hidden = true;
  }

  private async showSignedInAccount(): Promise<void> {
    const user = await fetchMe();
    this.account.textContent = typeof user.email === "string" ? user.email : this.t("signedIn");
    this.message.textContent = this.t("continueWith");
    this.signIn.hidden = true;
    this.passwordForm.hidden = true;
    this.modeTabs.hidden = true;
    this.proceed.hidden = false;
  }

  private async showSignedOut(): Promise<void> {
    this.message.textContent = this.t("signInToContinue");
    let google = false;
    try {
      google = (await fetchProviders()).google === true;
    } catch {
      /* Google stays hidden; password sign-in still works. */
    }
    this.signIn.hidden = !google;
    this.modeTabs.hidden = false;
    this.passwordForm.hidden = false;
  }

  private loadHandoff(): boolean {
    try {
      const fromLocation = readHandoffFromLocation();
      if (fromLocation) {
        this.pending = fromLocation;
        sessionStorage.setItem(HANDOFF_KEY, JSON.stringify(fromLocation));
        history.replaceState(null, "", "/desktop/login");
      } else {
        const stored: unknown = JSON.parse(sessionStorage.getItem(HANDOFF_KEY) ?? "null");
        this.pending = isValidHandoff(stored) ? stored : null;
      }
      return this.pending !== null;
    } catch {
      try {
        sessionStorage.removeItem(HANDOFF_KEY);
      } catch {
        /* Storage unavailable; the flow cannot continue. */
      }
      return false;
    }
  }

  private onCancel = (): void => {
    this.cancelled = true;
    try {
      sessionStorage.removeItem(HANDOFF_KEY);
    } catch {
      /* Already gone. */
    }
    this.cancel.hidden = true;
    this.fail(this.t("cancelled"));
  };

  private onGoogle = (): void => {
    location.assign("/api/auth/sign-in/social/google");
  };

  private onSubmit = async (event: SubmitEvent): Promise<void> => {
    event.preventDefault();
    if (this.mode === "register") await this.register();
    else await this.login();
  };

  private async login(): Promise<void> {
    const id = this.identifier.value.trim();
    if (!id || !this.password.value) return;
    this.message.textContent = this.t("signingIn");
    const credentials = id.includes("@")
      ? { email: id, password: this.password.value }
      : { username: id, password: this.password.value };
    try {
      await signIn(credentials);
      this.password.value = "";
      await this.showSignedInAccount();
    } catch {
      this.password.value = "";
      this.message.textContent = this.t("signInFailed");
    }
  }

  private async register(): Promise<void> {
    const name = this.nameInput.value.trim();
    const email = this.identifier.value.trim();
    if (!name || !email.includes("@") || !this.password.value) {
      this.message.textContent = this.t("needRegisterFields");
      return;
    }
    this.message.textContent = this.t("registering");
    try {
      await signUp({ email, name, password: this.password.value });
      this.password.value = "";
      await this.showSignedInAccount();
    } catch {
      this.password.value = "";
      this.message.textContent = this.t("registerFailed");
    }
  }

  private onContinue = async (): Promise<void> => {
    if (this.cancelled || !this.pending) return;
    this.proceed.disabled = true;
    try {
      const { callbackUrl } = await postHandoff(this.pending);
      if (this.cancelled) return;
      const checked = checkCallbackUrl(callbackUrl, this.pending);
      if (!checked) throw new Error("callback mismatch");
      try {
        sessionStorage.removeItem(HANDOFF_KEY);
      } catch {
        /* Already gone. */
      }
      location.assign(checked.href);
    } catch {
      if (!this.cancelled) {
        this.proceed.disabled = false;
        this.fail(this.t("handoffFailed"));
      }
    }
  };

  run(): void {
    this.renderLanguages();
    this.applyStrings();
    this.setMode("login");
    this.tabLogin.addEventListener("click", () => this.setMode("login"));
    this.tabRegister.addEventListener("click", () => this.setMode("register"));
    if (!this.loadHandoff()) {
      this.fail(this.t("badHandoff"));
      return;
    }
    this.cancel.hidden = false;
    this.cancel.addEventListener("click", this.onCancel);
    this.signIn.addEventListener("click", this.onGoogle);
    this.passwordForm.addEventListener("submit", this.onSubmit);
    this.proceed.addEventListener("click", this.onContinue);
    fetchMe()
      .then(async (user) => {
        if (this.cancelled) return;
        this.account.textContent =
          typeof user.email === "string" ? user.email : this.t("signedIn");
        this.message.textContent = this.t("continueWith");
        this.proceed.hidden = false;
      })
      .catch(async (error: unknown) => {
        if (this.cancelled) return;
        if (error instanceof ApiError && error.status === 401) {
          await this.showSignedOut();
          return;
        }
        this.fail(this.t("checkFailed"));
      });
  }
}

new ConsentPage().run();
