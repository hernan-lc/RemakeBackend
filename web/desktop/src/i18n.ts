export const LANGUAGES = ["en", "es"] as const;
export type Language = (typeof LANGUAGES)[number];

const en = {
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
} as const;

export type MessageKey = keyof typeof en;

const es: Record<MessageKey, string> = {
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
};

export const STRINGS: Record<Language, Record<MessageKey, string>> = { en, es };

const LANG_KEY = "tiktools.desktop.lang.v1";

export function detectLanguage(): Language {
  try {
    const stored = localStorage.getItem(LANG_KEY);
    if (stored === "en" || stored === "es") return stored;
  } catch {
    /* Private mode: fall through to browser detection without persisting. */
  }
  try {
    if ((navigator.language || "").toLowerCase().startsWith("es")) return "es";
  } catch {
    /* No navigator language available. */
  }
  return "en";
}

export function persistLanguage(lang: Language): void {
  try {
    localStorage.setItem(LANG_KEY, lang);
  } catch {
    /* Private mode: the choice lasts for this page load. */
  }
}
