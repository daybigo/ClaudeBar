// Modelo de proveedor (Claude / Codex / Antigravity).
// La parte testeable (isProvider, resolveProvider) es pura; load/save tocan
// localStorage. Mismo patrón que theme.ts.

export type Provider = "claude" | "codex" | "antigravity";

export const PROVIDERS: Provider[] = ["claude", "codex", "antigravity"];

export const PROVIDER_LABELS: Record<Provider, string> = {
  claude: "Claude",
  codex: "Codex",
  antigravity: "Antigravity",
};

const KEY = "provider";

export function isProvider(v: unknown): v is Provider {
  return v === "claude" || v === "codex" || v === "antigravity";
}

/** Dado el valor crudo guardado, devuelve un proveedor válido (claude por defecto). */
export function resolveProvider(stored: string | null): Provider {
  return isProvider(stored) ? stored : "claude";
}

export function loadProvider(): Provider {
  return resolveProvider(localStorage.getItem(KEY));
}

export function saveProvider(p: Provider): void {
  localStorage.setItem(KEY, p);
}
