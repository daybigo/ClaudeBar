// Lógica de temas (claro / oscuro / sistema).
// resolveTheme es pura y testeable; el resto toca el DOM/localStorage.

export type ThemeSetting = "light" | "dark" | "system";
export type EffectiveTheme = "light" | "dark";

const KEY = "theme";

/** Dado el ajuste del usuario y el tema del SO, devuelve el tema efectivo. */
export function resolveTheme(setting: ThemeSetting, prefersDark: boolean): EffectiveTheme {
  if (setting === "system") return prefersDark ? "dark" : "light";
  return setting;
}

export function loadThemeSetting(): ThemeSetting {
  const v = localStorage.getItem(KEY);
  return v === "light" || v === "dark" || v === "system" ? v : "system";
}

export function saveThemeSetting(s: ThemeSetting): void {
  localStorage.setItem(KEY, s);
}
