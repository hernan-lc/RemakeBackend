import "./style.css";
import {
  LANGUAGES,
  NATIVE_NAMES,
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
  private idField = el("id-field");
  private passwordField = el("password-field");
  private who = el("who");
  private passwordNext = el<HTMLButtonElement>("password-next");
  private passwordBack = el<HTMLButtonElement>("password-back");
  private passwordToggle = el<HTMLButtonElement>("password-toggle");
  private langButton = el<HTMLButtonElement>("lang-button");
  private langMenu = el("lang-menu");
  private langCurrent = el("lang-current");
  private step: 1 | 2 = 1;

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
    this.langCurrent.textContent = NATIVE_NAMES[this.lang];
    this.passwordNext.textContent = this.t("next");
    this.passwordBack.textContent = this.t("back");
    this.syncToggleLabel();
    this.langMenu.querySelectorAll<HTMLLIElement>("li").forEach((item) => {
      const selected = item.dataset.lang === this.lang;
      item.setAttribute("aria-selected", selected ? "true" : "false");
      item.classList.toggle("active", selected);
    });
  }

  private renderLanguages(): void {
    this.langMenu.replaceChildren();
    for (const code of LANGUAGES) {
      const item = document.createElement("li");
      item.dataset.lang = code;
      item.setAttribute("role", "option");
      item.tabIndex = -1;
      item.textContent = NATIVE_NAMES[code];
      item.addEventListener("click", () => {
        this.setLang(code);
        this.closeLangMenu();
      });
      this.langMenu.append(item);
    }
    this.langButton.addEventListener("click", (event) => {
      event.stopPropagation();
      this.langMenu.hidden ? this.openLangMenu() : this.closeLangMenu();
    });
    document.addEventListener("click", (event) => {
      if (!this.langMenu.hidden && !(event.target instanceof Node && el("langselect").contains(event.target))) {
        this.closeLangMenu();
      }
    });
    document.addEventListener("keydown", (event) => {
      if (event.key === "Escape" && !this.langMenu.hidden) this.closeLangMenu();
    });
  }

  private openLangMenu(): void {
    this.langMenu.hidden = false;
    this.langButton.setAttribute("aria-expanded", "true");
  }

  private closeLangMenu(): void {
    this.langMenu.hidden = true;
    this.langButton.setAttribute("aria-expanded", "false");
  }

  private setLang(next: Language): void {
    this.lang = next;
    persistLanguage(next);
    this.applyStrings();
  }

  private setMode(next: "login" | "register"): void {
    this.mode = next;
    this.tabLogin.classList.toggle("active", next === "login");
    this.tabRegister.classList.toggle("active", next === "register");
    this.passwordSignIn.textContent = this.t(next === "register" ? "createAccount" : "signIn");
    if (next === "register") {
      this.nameField.hidden = false;
      this.idField.hidden = false;
      this.passwordField.hidden = false;
      this.passwordNext.hidden = true;
      this.passwordBack.hidden = true;
      this.passwordSignIn.hidden = false;
      this.who.hidden = true;
    } else {
      this.nameField.hidden = true;
      this.showStep(1);
    }
  }

  private showStep(step: 1 | 2): void {
    this.step = step;
    const first = step === 1;
    this.idField.hidden = !first;
    this.passwordField.hidden = first;
    this.passwordNext.hidden = !first;
    this.passwordSignIn.hidden = first;
    this.passwordBack.hidden = first;
    this.who.hidden = first;
    if (!first) {
      this.who.textContent = this.identifier.value.trim();
      this.message.textContent = this.t("enterPassword");
    }
    this.syncToggleLabel();
  }

  private syncToggleLabel(): void {
    const showing = this.password.type === "text";
    this.passwordToggle.setAttribute("aria-pressed", showing ? "true" : "false");
    this.passwordToggle.setAttribute(
      "aria-label",
      showing ? this.t("hidePassword") : this.t("showPassword"),
    );
    this.passwordToggle.querySelector(".eye")?.toggleAttribute("hidden", showing);
    this.passwordToggle.querySelector(".eye-off")?.toggleAttribute("hidden", !showing);
  }

  private onTogglePassword = (): void => {
    this.password.type = this.password.type === "password" ? "text" : "password";
    this.syncToggleLabel();
    this.password.focus();
  };

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
    this.who.hidden = true;
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
    this.setMode(this.mode);
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

  private onNext = (): void => {
    if (this.mode !== "login" || this.step !== 1) return;
    if (!this.identifier.value.trim()) {
      this.identifier.focus();
      return;
    }
    this.showStep(2);
    this.password.focus();
  };

  private onBack = (): void => {
    if (this.mode !== "login" || this.step !== 2) return;
    this.message.textContent = this.t("signInToContinue");
    this.showStep(1);
    this.identifier.focus();
  };

  private onSubmit = async (event: SubmitEvent): Promise<void> => {
    event.preventDefault();
    if (this.mode === "register") {
      await this.register();
      return;
    }
    if (this.step === 1) {
      this.onNext();
      return;
    }
    await this.login();
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
    this.passwordNext.addEventListener("click", this.onNext);
    this.passwordBack.addEventListener("click", this.onBack);
    this.passwordToggle.addEventListener("click", this.onTogglePassword);
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
