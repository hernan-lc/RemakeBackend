export const LANGUAGES = ["en", "es", "fr", "de", "pt"] as const;
export type Language = (typeof LANGUAGES)[number];

export const NATIVE_NAMES: Record<Language, string> = {
  en: "English",
  es: "Español",
  fr: "Français",
  de: "Deutsch",
  pt: "Português",
};

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
  next: "Continue",
  back: "Back",
  showPassword: "Show password",
  hidePassword: "Hide password",
  enterPassword: "Enter your password to continue.",
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
  next: "Continuar",
  back: "Atrás",
  showPassword: "Mostrar contraseña",
  hidePassword: "Ocultar contraseña",
  enterPassword: "Escribe tu contraseña para continuar.",
};

const fr: Record<MessageKey, string> = {
  title: "TikTools Cloud : connexion",
  checking: "Vérification de la session…",
  signInToContinue: "Connectez-vous pour continuer vers TikTools.",
  identifier: "Nom d'utilisateur ou e-mail",
  name: "Nom affiché",
  password: "Mot de passe",
  signIn: "Se connecter",
  createAccount: "Créer un compte",
  signInGoogle: "Se connecter avec Google",
  continue: "Continuer vers TikTools",
  cancel: "Annuler",
  cancelled: "Connexion annulée. Vous pouvez fermer cette fenêtre.",
  badHandoff: "Ouvrez TikTools et recommencez la connexion.",
  checkFailed: "Impossible de vérifier la session. Réessayez.",
  signingIn: "Connexion…",
  signInFailed: "Échec de la connexion. Vérifiez l'identifiant et le mot de passe.",
  registering: "Création du compte…",
  registerFailed: "Impossible de créer le compte. L'e-mail est peut-être déjà enregistré.",
  needRegisterFields: "Saisissez un nom affiché, une adresse e-mail et un mot de passe.",
  continueWith: "Continuer vers TikTools avec ce compte ?",
  signedIn: "Connecté",
  handoffFailed: "Connexion incomplète. Recommencez dans TikTools.",
  next: "Continuer",
  back: "Retour",
  showPassword: "Afficher le mot de passe",
  hidePassword: "Masquer le mot de passe",
  enterPassword: "Saisissez votre mot de passe pour continuer.",
};

const de: Record<MessageKey, string> = {
  title: "TikTools Cloud: Anmeldung",
  checking: "Sitzung wird geprüft…",
  signInToContinue: "Melden Sie sich an, um mit TikTools fortzufahren.",
  identifier: "Benutzername oder E-Mail",
  name: "Anzeigename",
  password: "Passwort",
  signIn: "Anmelden",
  createAccount: "Konto erstellen",
  signInGoogle: "Mit Google anmelden",
  continue: "Weiter zu TikTools",
  cancel: "Abbrechen",
  cancelled: "Anmeldung abgebrochen. Sie können dieses Fenster schließen.",
  badHandoff: "Öffnen Sie TikTools und starten Sie die Anmeldung erneut.",
  checkFailed: "Sitzung konnte nicht geprüft werden. Versuchen Sie es erneut.",
  signingIn: "Anmeldung läuft…",
  signInFailed: "Anmeldung fehlgeschlagen. Prüfen Sie Kennung und Passwort.",
  registering: "Konto wird erstellt…",
  registerFailed: "Konto konnte nicht erstellt werden. Die E-Mail ist möglicherweise bereits registriert.",
  needRegisterFields: "Geben Sie einen Anzeigenamen, eine E-Mail-Adresse und ein Passwort ein.",
  continueWith: "Mit diesem Konto zu TikTools fortfahren?",
  signedIn: "Angemeldet",
  handoffFailed: "Anmeldung unvollständig. Starten Sie in TikTools neu.",
  next: "Weiter",
  back: "Zurück",
  showPassword: "Passwort anzeigen",
  hidePassword: "Passwort ausblenden",
  enterPassword: "Geben Sie Ihr Passwort ein, um fortzufahren.",
};

const pt: Record<MessageKey, string> = {
  title: "TikTools Cloud: entrar",
  checking: "Verificando a sessão…",
  signInToContinue: "Entre para continuar no TikTools.",
  identifier: "Usuário ou e-mail",
  name: "Nome de exibição",
  password: "Senha",
  signIn: "Entrar",
  createAccount: "Criar conta",
  signInGoogle: "Entrar com o Google",
  continue: "Continuar para o TikTools",
  cancel: "Cancelar",
  cancelled: "Entrada cancelada. Você pode fechar esta janela.",
  badHandoff: "Abra o TikTools e inicie a sessão novamente.",
  checkFailed: "Não foi possível verificar a sessão. Tente de novo.",
  signingIn: "Entrando…",
  signInFailed: "Falha na entrada. Verifique o identificador e a senha.",
  registering: "Criando a conta…",
  registerFailed: "Não foi possível criar a conta. O e-mail já pode estar registrado.",
  needRegisterFields: "Digite um nome de exibição, um e-mail e uma senha.",
  continueWith: "Continuar para o TikTools com esta conta?",
  signedIn: "Conectado",
  handoffFailed: "Não foi possível concluir. Recomece no TikTools.",
  next: "Continuar",
  back: "Voltar",
  showPassword: "Mostrar senha",
  hidePassword: "Ocultar senha",
  enterPassword: "Digite sua senha para continuar.",
};

export const STRINGS: Record<Language, Record<MessageKey, string>> = { en, es, fr, de, pt };

const LANG_KEY = "tiktools.desktop.lang.v1";

export function detectLanguage(): Language {
  try {
    const stored = localStorage.getItem(LANG_KEY);
    if (stored && (LANGUAGES as readonly string[]).includes(stored)) return stored as Language;
  } catch {
    /* Private mode: fall through to browser detection without persisting. */
  }
  try {
    const browser = (navigator.language || "").toLowerCase();
    const match = LANGUAGES.find((code) => browser === code || browser.startsWith(`${code}-`));
    if (match) return match;
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
