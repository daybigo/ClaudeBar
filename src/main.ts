import "./styles.css";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { getVersion } from "@tauri-apps/api/app";
import { openUrl } from "@tauri-apps/plugin-opener";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { LogicalSize, LogicalPosition } from "@tauri-apps/api/dpi";
import { resolveTheme, loadThemeSetting, saveThemeSetting, type ThemeSetting } from "./theme";
import { loadProvider, saveProvider, PROVIDER_LABELS, type Provider } from "./provider";

// ----- Tipos (coinciden con el backend, camelCase) -----
interface LimitWindow {
  utilization: number;
  resetsAt: string | null;
  resetsInLabel: string;
}
interface ScopedWindow {
  model: string;
  modelId: string;
  utilization: number;
  resetsAt: string | null;
  resetsInLabel: string;
}
interface ExtraUsage {
  usedUsd: number;
  limitUsd: number;
  utilization: number;
}
interface UsageSnapshot {
  connected: boolean;
  plan: string;
  fiveHour: LimitWindow;
  sevenDay: LimitWindow;
  sevenDaySonnet: LimitWindow | null;
  sevenDayOpus: LimitWindow | null;
  scopedWeekly: ScopedWindow[];
  extraUsage: ExtraUsage;
  stale: boolean;
  error: string | null;
  updatedAt: string;
}
interface ModelUsage {
  model: string;
  costUsd: number;
  tokens: number;
}
interface CostReport {
  todayUsd: number;
  todayTokens: number;
  weekUsd: number;
  weekTokens: number;
  monthUsd: number;
  monthTokens: number;
  last30Usd: number;
  last30Tokens: number;
  daily: number[];
  topModel: string;
  models: ModelUsage[];
  updatedAt: string;
  empty: boolean;
}
interface CodexCost {
  todayUsd: number;
  last30Usd: number;
  monthTokens: number;
  weekTokens: number;
  last30Tokens: number;
  daily: number[];
  models: { model: string; tokens: number; costUsd: number | null }[];
  unpricedTokens: number;
  updatedAt: string;
  empty: boolean;
  incomplete: boolean;
}

// ----- i18n -----
type Dict = Record<string, string>;
const I18N: Record<string, Dict> = {
  es: {
    session: "Sesión", weekly: "Semanal", extra: "Uso extra", cost: "Costo",
    dashboard: "Panel de uso", status: "Estado del servicio", refresh: "Actualizar ahora",
    settings: "Ajustes", about: "Acerca de", logout: "Cerrar sesión (Claude)", quit: "Cerrar aplicación",
    used: "usado", resetsIn: "Reinicia en", pace: "Ritmo",
    behind: "Por debajo del ritmo", ahead: "Por encima del ritmo", onpace: "En ritmo",
    today: "Hoy", week: "Semana", last30: "Últimos 30 días", tokens: "tokens",
    costNote: "≈ valor equivalente en API · tu plan lo cubre",
    gToday: "Hoy", g30: "30 días", gMonthTok: "Tokens (mes)", gWeekTok: "Tokens (sem)", topModel: "Modelo top",
    byModel: "Uso por modelo (30 días)",
    codexCostNote: "≈ equivalente API estándar · estimación, no cargo real",
    codexScope: "Historial local de este equipo · tokens incluyen caché",
    codexPriceBasis: "Tarifa base; no incluye recargos por velocidad, contexto largo ni herramientas.",
    codexEmpty: "Aún no hay consumo registrado en los últimos 30 días.",
    codexPartial: "Estimación parcial: hay modelos sin tarifa conocida.",
    codexIncomplete: "No se pudo leer parte del historial local.",
    tokenShare: "Participación en tokens · 30 días",
    noPrice: "Sin tarifa",
    credits: "Créditos extra", balance: "Saldo disponible", unlimited: "Sin límite",
    localLimits: "Límites del último registro local", savedLimits: "Última consulta guardada",
    thisMonth: "Este mes", updatedJust: "actualizado recién", ago: "hace",
    connect: "Conecta Claude Code para ver tu uso", langBtn: "English",
    errExpired: "Sesión expirada — abre Claude Code para renovar",
    errRate: "Muchas consultas, reintentando…",
    errNetwork: "Sin conexión, reintentando…",
    errParse: "Respuesta inesperada", errGeneric: "Error temporal, reintentando…",
    theme: "Tema", themeLight: "Claro", themeDark: "Oscuro", themeSystem: "Sistema",
    connected: "conectado", notConnected: "no conectado",
    open: "Abrir", loadingProvider: "Cargando…", monthly: "Mes",
    usageNotHere: "Sin datos de uso para este proveedor.",
    openAntigravity: "Abre la app de Antigravity para ver tu uso.",
    connectHint: "Inicia sesión en {p} para conectarlo.",
    aboutTitle: "Acerca de Claude Bar", settingsTitle: "Ajustes", logoutTitle: "Cerrar sesión (Claude)",
  },
  en: {
    session: "Session", weekly: "Weekly", extra: "Extra usage", cost: "Cost",
    dashboard: "Usage Dashboard", status: "Status Page", refresh: "Refresh now",
    settings: "Settings", about: "About", logout: "Log out (Claude)", quit: "Quit",
    used: "used", resetsIn: "Resets in", pace: "Pace",
    behind: "Behind pace", ahead: "Ahead of pace", onpace: "On pace",
    today: "Today", week: "Week", last30: "Last 30 days", tokens: "tokens",
    costNote: "≈ API-equivalent value · covered by your plan",
    gToday: "Today", g30: "30 days", gMonthTok: "Tokens (month)", gWeekTok: "Tokens (week)", topModel: "Top model",
    byModel: "By model (30 days)",
    codexCostNote: "≈ standard API equivalent · estimate, not an actual charge",
    codexScope: "Local history on this computer · tokens include cache",
    codexPriceBasis: "Base rates; excludes speed, long-context and tool surcharges.",
    codexEmpty: "No usage recorded in the last 30 days yet.",
    codexPartial: "Partial estimate: some models have no known price.",
    codexIncomplete: "Some local history could not be read.",
    tokenShare: "Share of tokens · 30 days",
    noPrice: "No price",
    credits: "Extra credits", balance: "Available balance", unlimited: "Unlimited",
    localLimits: "Limits from the last local record", savedLimits: "Last saved lookup",
    thisMonth: "This month", updatedJust: "updated just now", ago: "ago",
    connect: "Connect Claude Code to see your usage", langBtn: "Español",
    errExpired: "Session expired — open Claude Code to renew",
    errRate: "Too many requests, retrying…",
    errNetwork: "No connection, retrying…",
    errParse: "Unexpected response", errGeneric: "Temporary error, retrying…",
    theme: "Theme", themeLight: "Light", themeDark: "Dark", themeSystem: "System",
    connected: "connected", notConnected: "not connected",
    open: "Open", loadingProvider: "Loading…", monthly: "Month",
    usageNotHere: "No usage data for this provider.",
    openAntigravity: "Open the Antigravity app to see your usage.",
    connectHint: "Sign in to {p} to connect it.",
    aboutTitle: "About Claude Bar", settingsTitle: "Settings", logoutTitle: "Log out (Claude)",
  },
};
// Primer arranque: sin preferencia guardada, seguimos el idioma del SO.
// WebView2 expone el locale del sistema en navigator.language(s), así que si
// el Windows del usuario está en español arranca en ES; si no, en EN.
function initialLang(): "es" | "en" {
  const saved = localStorage.getItem("lang");
  if (saved === "es" || saved === "en") return saved;
  const tags =
    navigator.languages && navigator.languages.length
      ? navigator.languages
      : [navigator.language || ""];
  return tags.some((l) => l.toLowerCase().startsWith("es")) ? "es" : "en";
}
let lang: "es" | "en" = initialLang();
const t = (k: string) => I18N[lang][k] ?? k;
function errText(code: string): string {
  const m: Record<string, string> = {
    session_expired: t("errExpired"),
    rate_limited: t("errRate"),
    network: t("errNetwork"),
    parse_error: t("errParse"),
  };
  return m[code] ?? t("errGeneric");
}

// ----- Tema (claro / oscuro / sistema) -----
function prefersDark(): boolean {
  return window.matchMedia("(prefers-color-scheme: dark)").matches;
}
function applyTheme(setting: ThemeSetting): void {
  document.documentElement.setAttribute("data-theme", resolveTheme(setting, prefersDark()));
}
function markThemeSelection(setting: ThemeSetting): void {
  document.querySelectorAll<HTMLElement>("[data-theme-opt]").forEach((el) => {
    el.classList.toggle("active", el.dataset.themeOpt === setting);
  });
}
function setTheme(setting: ThemeSetting): void {
  saveThemeSetting(setting);
  applyTheme(setting);
  markThemeSelection(setting);
}

// ----- Proveedor (Claude / Codex / Antigravity) -----
function markProviderTab(p: Provider): void {
  document.querySelectorAll<HTMLElement>("[data-provider-tab]").forEach((el) => {
    el.classList.toggle("active", el.dataset.providerTab === p);
  });
}
interface UsageWindow {
  usedPercent: number;
  windowMinutes: number;
  resetsAt: number; // epoch en segundos
}
interface AntigravityBucket {
  group: string;
  label: string;
  window: string; // "5h" | "weekly"
  usedPercent: number;
  resetsAt: number;
}
interface ProviderStatus {
  connected: boolean;
  email: string;
  plan: string;
  primary?: UsageWindow | null;
  secondary?: UsageWindow | null;
  buckets?: AntigravityBucket[];
  additional?: { label: string; primary: UsageWindow | null; secondary: UsageWindow | null }[];
  credits?: { balance: number | null; unlimited: boolean; hasCredits: boolean } | null;
  cost?: CodexCost | null;
  usageSource?: string;
  usageUpdatedAt?: string;
  usageError?: string | null;
}
function bucketLabel(b: AntigravityBucket): string {
  if (b.window === "5h") return t("session");
  if (b.window === "weekly") return t("weekly");
  return b.label || b.window;
}
function windowLabel(mins: number): string {
  if (mins <= 360) return t("session");
  if (mins <= 11000) return t("weekly");
  return t("monthly");
}
function resetLabel(epochSec: number): string {
  if (!epochSec) return "";
  const ms = epochSec * 1000 - Date.now();
  if (ms <= 0) return "";
  const totalMin = Math.floor(ms / 60000);
  const d = Math.floor(totalMin / 1440);
  const h = Math.floor((totalMin % 1440) / 60);
  const m = totalMin % 60;
  if (d >= 1) return `${d}d ${h}h`;
  if (h >= 1) return `${h}h ${m}m`;
  return `${m}m`;
}
// Bloque de uso con el MISMO estilo que Claude: etiqueta, barra, y abajo
// "% usado" a la izquierda + "Reinicia en" a la derecha.
function usageBlock(label: string, pct: number, resetEpoch: number, pace = ""): string {
  const p = Math.max(0, Math.min(100, pct));
  const reset = resetLabel(resetEpoch);
  return `<section class="block">
      <h2>${esc(label)}</h2>
      <div class="bar"><div class="fill" style="width:${p}%"></div></div>
      <div class="bar-foot"><span class="muted">${fmtPct(pct)} ${t("used")}</span><span class="muted right">${
        reset ? `${t("resetsIn")} ${reset}` : ""
      }</span></div>
      ${pace ? `<div class="pace">${pace}</div>` : ""}
    </section>`;
}
// Ritmo de una ventana externa: compara lo usado con lo esperado segun el
// tiempo transcurrido de la ventana. Solo tiene sentido en ventanas largas.
function windowPace(w: UsageWindow): string {
  if (!w.windowMinutes || !w.resetsAt) return "";
  const totalMs = w.windowMinutes * 60000;
  const remainMs = w.resetsAt * 1000 - Date.now();
  if (remainMs <= 0 || remainMs > totalMs) return "";
  const expected = ((totalMs - remainMs) / totalMs) * 100;
  const delta = w.usedPercent - expected;
  const label = delta < -2 ? t("behind") : delta > 2 ? t("ahead") : t("onpace");
  const sign = delta > 0 ? "+" : "";
  return `${t("pace")}: ${label} (${sign}${delta.toFixed(0)}%)`;
}
function usageBars(st: ProviderStatus): string {
  const wins = [st.primary, st.secondary].filter(Boolean) as UsageWindow[];
  return wins
    .map((w) => {
      const pace = w.windowMinutes > 360 ? windowPace(w) : ""; // solo ventanas > 6h
      return usageBlock(windowLabel(w.windowMinutes), w.usedPercent, w.resetsAt, pace);
    })
    .join("");
}
function antigravityBars(buckets: AntigravityBucket[]): string {
  if (!buckets.length) return "";
  const groups: { name: string; items: AntigravityBucket[] }[] = [];
  for (const b of buckets) {
    let g = groups.find((x) => x.name === b.group);
    if (!g) {
      g = { name: b.group, items: [] };
      groups.push(g);
    }
    g.items.push(b);
  }
  return groups
    .map(
      (g) =>
        `<div class="pgroup">${esc(g.name)}</div>` +
        g.items.map((b) => usageBlock(bucketLabel(b), b.usedPercent, b.resetsAt)).join("")
    )
    .join("");
}
// Proveedores externos (Codex/Antigravity): muestran una tarjeta de cuenta.
const EXTERNAL_CMD: Partial<Record<Provider, string>> = {
  codex: "get_codex",
  antigravity: "get_antigravity",
};
const PROVIDER_COLOR: Record<Provider, string> = {
  claude: "#f0a24a",
  codex: "#10a37f", // verde OpenAI
  antigravity: "#4285f4", // azul Google
};
const PROVIDER_OPEN: Partial<Record<Provider, string>> = {
  codex: "https://chatgpt.com",
  antigravity: "https://antigravity.google",
};
const esc = (s: string) => s.replace(/[&<>"]/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;" }[c]!));

function codexMetrics(st: ProviderStatus): string {
  const extraLimits = (st.additional || []).map((limit) => {
    const bars = usageBars({ connected: true, email: "", plan: "", ...limit });
    return bars ? `<div class="pgroup">${esc(limit.label)}</div>${bars}` : "";
  }).join("");
  const credits = st.credits
    ? `<hr class="rule" /><section class="block"><h2>${t("credits")}</h2>
        <div class="bar-foot"><span class="muted">${t("balance")}</span>
        <span class="muted">${st.credits.unlimited ? t("unlimited") : st.credits.balance === null ? "—" :
          new Intl.NumberFormat(lang, { maximumFractionDigits: 2 }).format(st.credits.balance)}</span></div></section>`
    : "";
  const c = st.cost;
  let cost = `<p class="pnote">${t("loadingProvider")}</p>`;
  if (c) {
    const models = c.models.map((m) => {
      const share = c.last30Tokens > 0 ? m.tokens / c.last30Tokens * 100 : 0;
      return `<div class="codex-model"><div class="mb-row">
        <span class="mb-name">${esc(prettyModel(m.model))}</span>
        <span class="mb-tok">${fmtTokens(m.tokens)}</span>
        <span class="mb-cost">${m.costUsd === null ? t("noPrice") : fmtUsd(m.costUsd)}</span></div>
        <div class="bar" role="meter" aria-label="${esc(prettyModel(m.model))}: ${t("tokenShare")}" aria-valuemin="0" aria-valuemax="100" aria-valuenow="${share.toFixed(1)}"
          title="${share.toFixed(1)}% · ${t("tokenShare")}"><div class="fill" style="width:${share}%"></div></div></div>`;
    }).join("");
    const hasPrices = c.models.some((m) => m.costUsd !== null);
    const money = (n: number) => c.empty ? "—" : hasPrices ? `${fmtUsd(n)}${c.unpricedTokens ? "*" : ""}` : "—";
    cost = `<hr class="rule" /><section class="block cost">
      <h2>${t("cost")}</h2>
      <div class="cost-grid">
        <div class="cg-cell"><div class="cg-label">${t("gToday")}</div><div class="cg-value">${money(c.todayUsd)}</div></div>
        <div class="cg-cell"><div class="cg-label">${t("g30")}</div><div class="cg-value">${money(c.last30Usd)}</div></div>
        <div class="cg-cell"><div class="cg-label">${t("gMonthTok")}</div><div class="cg-value">${fmtTokens(c.monthTokens)}</div></div>
        <div class="cg-cell"><div class="cg-label">${t("gWeekTok")}</div><div class="cg-value">${fmtTokens(c.weekTokens)}</div></div>
      </div>
      ${hasPrices ? `<div class="chart">${chartBars(c.daily)}</div>` : ""}
      ${c.empty ? `<p class="pnote">${t("codexEmpty")}</p>` : `<div class="model-breakdown"><div class="mb-head">${t("byModel")}</div>${models}
        <div class="cost-line subtle">${t("tokenShare")}</div></div>`}
      <div class="cost-line subtle" title="${t("codexPriceBasis")}">${t("codexCostNote")}</div>
      <div class="cost-line subtle">${t("codexScope")}</div>
      ${c.unpricedTokens ? `<div class="cost-line subtle">* ${t("codexPartial")}</div>` : ""}
      ${c.incomplete ? `<div class="cost-line subtle">${t("codexIncomplete")}</div>` : ""}
    </section>`;
  }
  return `${extraLimits}${credits}${cost}`;
}

function providerCard(p: Provider, st: ProviderStatus): string {
  const label = PROVIDER_LABELS[p];
  const initial = esc(label.charAt(0));
  if (!st.connected) {
    return `<div class="pcard">
        <div class="pemblem off">${initial}</div>
        <div class="pplan">${t("notConnected")}</div>
        <p class="pnote">${t("connectHint").replace("{p}", esc(label))}</p>
      </div>`;
  }
  const url = PROVIDER_OPEN[p];
  const bars = st.buckets && st.buckets.length ? antigravityBars(st.buckets) : usageBars(st);
  // Cabecera de cuenta del dashboard: emblema + plan + email.
  const head = `<div class="pdash-head">
      <div class="pemblem sm" style="background:${PROVIDER_COLOR[p]}">${initial}</div>
      <div class="pdash-id">
        <div class="pdash-plan">${esc(st.plan || label)}</div>
        ${st.email ? `<div class="pdash-email">${esc(st.email)}</div>` : ""}
      </div>
    </div>`;
  const openBtn = url
    ? `<button class="pcard-btn wide" data-act="open:${url}">${t("open")} ${esc(label)} ↗</button>`
    : "";
  if (p === "codex") {
    const source = st.usageSource === "local" ? t("localLimits") : st.usageError ? t("savedLimits") : "";
    return `<div class="pdash">${head}<hr class="rule" />${bars || `<p class="pnote">${t("usageNotHere")}</p>`}
      ${source ? `<div class="cost-line subtle">${source}${st.usageUpdatedAt ? ` · ${relTime(st.usageUpdatedAt)}` : ""}</div>` : ""}
      ${st.usageError ? `<div class="cost-line subtle">${esc(errText(st.usageError))}</div>` : ""}
      ${codexMetrics(st)}<hr class="rule" />
      <section class="actions">
        <button class="action" data-act="open:https://chatgpt.com/codex/settings/usage"><span class="ic">▥</span>${t("dashboard")}</button>
        <button class="action" data-act="open:https://status.openai.com"><span class="ic">⟋</span>${t("status")}</button>
        <button class="action" data-act="refresh-codex"><span class="ic">⟳</span>${t("refresh")}</button>
      </section>${openBtn}</div>`;
  }
  if (bars) {
    // Dashboard completo: cuenta + medidores (mismo estilo que Claude) + abrir.
    return `<div class="pdash">${head}<hr class="rule" />${bars}${openBtn}</div>`;
  }
  // Conectado pero sin medidores (p.ej. app de Antigravity cerrada).
  return `<div class="pdash">${head}
      <p class="pnote">${p === "antigravity" ? t("openAntigravity") : t("usageNotHere")}</p>
      ${openBtn}</div>`;
}

async function refreshExternal(p: Provider, command: string): Promise<void> {
  let st: ProviderStatus = { connected: false, email: "", plan: "" };
  try {
    st = await invoke<ProviderStatus>(command);
  } catch (e) {
    console.error(command, e);
  }
  applyExternal(p, st);
}

function applyExternal(p: Provider, st: ProviderStatus): void {
  const usageVals: number[] = [];
  if (st.primary) usageVals.push(st.primary.usedPercent);
  if (st.secondary) usageVals.push(st.secondary.usedPercent);
  for (const b of st.buckets || []) usageVals.push(b.usedPercent);
  setMeter(p, usageVals.length ? Math.max(...usageVals) : 0);

  if (loadProvider() !== p) return; // el usuario cambió mientras tanto
  $("plan-badge").textContent = st.connected ? st.plan : "";
  const updated = $("updated");
  updated.classList.toggle("stale", Boolean(st.usageError));
  updated.textContent = st.connected
    ? st.email
      ? `${t("connected")} · ${st.email}`
      : t("connected")
    : t("notConnected");
  $("soon").innerHTML = providerCard(p, st);
}

function applyProvider(p: Provider): void {
  markProviderTab(p);
  $("provider-title").textContent = PROVIDER_LABELS[p];
  const isClaude = p === "claude";
  $("data-sections").classList.toggle("hidden", !isClaude);
  $("soon").classList.toggle("hidden", isClaude);
  if (isClaude) {
    if (lastUsage) applyUsage(lastUsage);
    return;
  }
  const cmd = EXTERNAL_CMD[p];
  if (cmd) {
    $("plan-badge").textContent = "";
    $("soon").innerHTML = `<div class="pcard"><p class="pnote">${t("loadingProvider")}</p></div>`;
    void refreshExternal(p, cmd);
  }
}
function setProvider(p: Provider): void {
  saveProvider(p);
  applyProvider(p);
  // Sincroniza la bandeja de Windows con el proveedor elegido.
  void invoke("set_provider", { provider: p }).catch((e) => console.error("set_provider:", e));
}

const $ = (id: string) => document.getElementById(id)!;
const appWindow = getCurrentWindow();
const FULL = { w: 384, h: 700 };
const COMPACT = { w: 228, h: 112 };

let lastUsage: UsageSnapshot | null = null;
let lastCost: CostReport | null = null;
let lastUpdatedIso = "";
let lastPlan = "";

// ----- Formato -----
function fmtUsd(n: number): string {
  return "$ " + n.toLocaleString("en-US", { minimumFractionDigits: 2, maximumFractionDigits: 2 });
}
function fmtTokens(n: number): string {
  if (n >= 1_000_000) return (n / 1_000_000).toFixed(n >= 10_000_000 ? 0 : 1) + "M";
  if (n >= 1_000) return Math.round(n / 1_000) + "K";
  return String(n);
}
function fmtPct(n: number): string {
  return (n < 10 ? n.toFixed(n < 1 ? 1 : 0) : Math.round(n).toString()) + "%";
}
function relTime(iso: string): string {
  if (!iso) return "";
  const then = new Date(iso).getTime();
  if (isNaN(then)) return "";
  const s = Math.floor(Math.max(0, Date.now() - then) / 1000);
  if (s < 8) return t("updatedJust");
  const val = s < 60 ? `${s}s` : s < 3600 ? `${Math.floor(s / 60)} min` : `${Math.floor(s / 3600)} h`;
  return lang === "es" ? `${t("ago")} ${val}` : `${val} ${t("ago")}`;
}
function setBar(id: string, util: number) {
  ($(id) as HTMLElement).style.width = Math.max(0, Math.min(100, util)) + "%";
}
function weeklyPace(win: LimitWindow): string {
  if (!win.resetsAt) return "";
  const end = new Date(win.resetsAt).getTime();
  if (isNaN(end)) return "";
  const windowMs = 7 * 24 * 3600 * 1000;
  const frac = Math.max(0, Math.min(1, (windowMs - (end - Date.now())) / windowMs));
  const delta = win.utilization - frac * 100;
  const sign = delta >= 0 ? "+" : "";
  const label = delta < -2 ? t("behind") : delta > 2 ? t("ahead") : t("onpace");
  return `${t("pace")}: ${label} (${sign}${delta.toFixed(0)}%)`;
}

// ----- Pintado -----
function setMeter(p: Provider, pct: number) {
  const el = document.getElementById(`meter-${p}`);
  if (el) el.style.width = `${Math.max(0, Math.min(100, pct))}%`;
}

// Barras de ventana semanal por modelo (Fable, Opus, Sonnet...). Prefiere el
// array scopedWeekly del endpoint (limits[]); si viene vacio, cae a las
// ventanas planas legacy seven_day_sonnet/opus.
function renderModelWindows(u: UsageSnapshot): string {
  const wins: { label: string; util: number; reset: string }[] = [];
  if (u.scopedWeekly && u.scopedWeekly.length) {
    for (const w of u.scopedWeekly) {
      wins.push({ label: w.model, util: w.utilization, reset: w.resetsInLabel });
    }
  } else {
    if (u.sevenDaySonnet) {
      wins.push({ label: "Sonnet", util: u.sevenDaySonnet.utilization, reset: u.sevenDaySonnet.resetsInLabel });
    }
    if (u.sevenDayOpus) {
      wins.push({ label: "Opus", util: u.sevenDayOpus.utilization, reset: u.sevenDayOpus.resetsInLabel });
    }
  }
  return wins
    .map((w) => {
      const p = Math.max(0, Math.min(100, w.util));
      return (
        `<section class="block"><h2>${esc(w.label)}</h2>` +
        `<div class="bar"><div class="fill" style="width:${p}%"></div></div>` +
        `<div class="bar-foot"><span class="muted">${fmtPct(w.util)} ${t("used")}</span>` +
        `<span class="muted right">${w.reset ? `${t("resetsIn")} ${w.reset}` : ""}</span></div></section>`
      );
    })
    .join("");
}

function applyUsage(u: UsageSnapshot) {
  lastUsage = u;
  lastPlan = u.connected ? u.plan : "";
  setMeter("claude", u.connected ? u.fiveHour.utilization : 0);
  // Cacheamos siempre, pero solo pintamos si Claude es el proveedor activo.
  if (loadProvider() !== "claude") return;
  $("plan-badge").textContent = lastPlan;

  const updated = $("updated");
  if (!u.connected) {
    updated.textContent = t("connect");
    updated.classList.add("stale");
  } else if (u.error && u.stale) {
    updated.textContent = u.plan ? `${u.plan} · ${errText(u.error)}` : errText(u.error);
    updated.classList.add("stale");
  } else {
    lastUpdatedIso = u.updatedAt;
    updated.textContent = `${u.plan} · ${relTime(u.updatedAt)}`;
    updated.classList.remove("stale");
  }

  // Sesión (5h)
  setBar("session-fill", u.fiveHour.utilization);
  $("session-pct").textContent = `${fmtPct(u.fiveHour.utilization)} ${t("used")}`;
  $("session-reset").textContent = u.fiveHour.resetsInLabel
    ? `${t("resetsIn")} ${u.fiveHour.resetsInLabel}`
    : "";
  setBar("cv-session-fill", u.fiveHour.utilization);
  $("cv-session-pct").textContent = fmtPct(u.fiveHour.utilization);

  // Semanal (7d)
  setBar("weekly-fill", u.sevenDay.utilization);
  $("weekly-pct").textContent = `${fmtPct(u.sevenDay.utilization)} ${t("used")}`;
  $("weekly-reset").textContent = u.sevenDay.resetsInLabel
    ? `${t("resetsIn")} ${u.sevenDay.resetsInLabel}`
    : "";
  $("weekly-pace").textContent = weeklyPace(u.sevenDay);
  setBar("cv-weekly-fill", u.sevenDay.utilization);
  $("cv-weekly-pct").textContent = fmtPct(u.sevenDay.utilization);

  // Ventanas semanales por modelo (Sonnet / Opus / Fable...): dinamicas.
  $("model-windows").innerHTML = renderModelWindows(u);

  // Uso extra (se oculta si nunca se usó nada)
  const ex = u.extraUsage;
  const extraUsed = ex.usedUsd > 0 || ex.utilization > 0;
  $("extra-block").style.display = extraUsed ? "" : "none";
  $("extra-hr").style.display = extraUsed ? "" : "none";
  setBar("extra-fill", ex.utilization);
  $("extra-amount").textContent = `${t("thisMonth")}: ${fmtUsd(ex.usedUsd)} / ${fmtUsd(ex.limitUsd)}`;
  $("extra-pct").textContent = `${fmtPct(ex.utilization)} ${t("used")}`;
}

function chartBars(daily: number[]): string {
  if (!daily.length) return "";
  const max = Math.max(0.0001, ...daily);
  return daily
    .map((v) => {
      const h = v > 0 ? Math.max(6, Math.round((v / max) * 100)) : 0;
      return `<div class="chart-bar" style="height:${h}%" title="$ ${v.toFixed(2)}"></div>`;
    })
    .join("");
}
function renderChart(daily: number[]) {
  $("cost-chart").innerHTML = chartBars(daily);
}
// Nombre bonito del modelo: "claude-fable-5" -> "Fable 5", "claude-opus-4-8" -> "Opus 4.8".
function prettyModel(id: string): string {
  const m = id.toLowerCase();
  if (m.startsWith("gpt-")) return id.replace(/^gpt-/i, "GPT-").replace(/-(astra|sol|terra|luna|codex|spark|mini|nano|pro)/gi,
    (_, name: string) => ` ${name.charAt(0).toUpperCase()}${name.slice(1)}`);
  const fam = ["opus", "sonnet", "haiku", "fable"].find((f) => m.includes(f));
  if (!fam) return id;
  const nums = (m.split(fam)[1] || "").match(/\d+/g)?.filter((n) => n.length <= 2) ?? [];
  const label = fam.charAt(0).toUpperCase() + fam.slice(1);
  return nums.length ? `${label} ${nums.join(".")}` : label;
}
// Desglose de costo/tokens por modelo (30 dias). Muestra Fable, Opus, etc.
function renderModels(models: ModelUsage[]): string {
  const rows = (models || []).filter((m) => m.costUsd > 0 || m.tokens > 0).slice(0, 5);
  if (!rows.length) return "";
  return (
    `<div class="mb-head">${t("byModel")}</div>` +
    rows
      .map(
        (m) =>
          `<div class="mb-row"><span class="mb-name">${esc(prettyModel(m.model))}</span>` +
          `<span class="mb-tok">${fmtTokens(m.tokens)}</span>` +
          `<span class="mb-cost">${fmtUsd(m.costUsd)}</span></div>`
      )
      .join("")
  );
}
function applyCost(c: CostReport) {
  lastCost = c;
  renderChart(c.daily || []);
  $("model-breakdown").innerHTML = renderModels(c.models);
  // El desglose ya encabeza con el modelo top; dejamos la linea suelta vacia.
  $("top-model").textContent = "";
  $("cg-today").textContent = fmtUsd(c.todayUsd);
  $("cg-30").textContent = fmtUsd(c.last30Usd);
  $("cg-month-tok").textContent = fmtTokens(c.monthTokens);
  $("cg-week-tok").textContent = fmtTokens(c.weekTokens);
  $("cost-note").textContent = t("costNote");
}

// ----- Idioma -----
function applyLang() {
  document.querySelectorAll<HTMLElement>("[data-i18n]").forEach((el) => {
    el.textContent = t(el.dataset.i18n || "");
  });
  $("lang-label").textContent = t("langBtn");
  document.documentElement.lang = lang;
  if (lastUsage) applyUsage(lastUsage);
  if (lastCost) applyCost(lastCost);
  applyProvider(loadProvider());
}
function toggleLang() {
  lang = lang === "es" ? "en" : "es";
  localStorage.setItem("lang", lang);
  applyLang();
}

// ----- Modal -----
function openModal(title: string, html: string) {
  $("modal-title").textContent = title;
  $("modal-body").innerHTML = html;
  $("modal").classList.remove("hidden");
}
function closeModal() {
  $("modal").classList.add("hidden");
}
async function showAbout() {
  const v = await getVersion();
  const body =
    lang === "es"
      ? `<p><b>Claude Bar</b> — monitor de uso de Claude para Windows, en tu bandeja.</p>
         <p>Proyecto creado por <b>Daybi</b>.</p>
         <p>Open source · build in public.</p>
         <p class="muted2">Versión ${v} · Rust + Tauri</p>`
      : `<p><b>Claude Bar</b> — Claude usage monitor for Windows, in your tray.</p>
         <p>Created by <b>Daybi</b>.</p>
         <p>Open source · build in public.</p>
         <p class="muted2">Version ${v} · Rust + Tauri</p>`;
  openModal(t("aboutTitle"), body);
}
async function showSettings() {
  const v = await getVersion();
  const themeRow = `<div class="row"><span>${t("theme")}</span>
       <span class="seg">
         <button class="seg-btn" data-theme-opt="light" data-act="theme:light">${t("themeLight")}</button>
         <button class="seg-btn" data-theme-opt="dark" data-act="theme:dark">${t("themeDark")}</button>
         <button class="seg-btn" data-theme-opt="system" data-act="theme:system">${t("themeSystem")}</button>
       </span></div>`;
  const rows =
    lang === "es"
      ? `<div class="row"><span>Versión</span><span class="muted2">${v}</span></div>
         <div class="row"><span>Cuenta</span><span class="muted2">Claude Code (local)</span></div>
         <div class="row"><span>Inicio con Windows</span><span class="muted2">menú del icono</span></div>
         <div class="row"><span>Refresco de uso</span><span class="muted2">5 min</span></div>
         <div class="row"><span>Refresco de costo</span><span class="muted2">60 s</span></div>
         <p class="muted2" style="margin-top:12px">Arrastra la barra superior para mover la ventana.</p>`
      : `<div class="row"><span>Version</span><span class="muted2">${v}</span></div>
         <div class="row"><span>Account</span><span class="muted2">Claude Code (local)</span></div>
         <div class="row"><span>Start with Windows</span><span class="muted2">tray menu</span></div>
         <div class="row"><span>Usage refresh</span><span class="muted2">5 min</span></div>
         <div class="row"><span>Cost refresh</span><span class="muted2">60 s</span></div>
         <p class="muted2" style="margin-top:12px">Drag the top bar to move the window.</p>`;
  openModal(t("settingsTitle"), themeRow + rows);
  markThemeSelection(loadThemeSetting());
}
function showLogout() {
  const body =
    lang === "es"
      ? `<p>Claude Bar usa la sesión local de <b>Claude Code</b> en tu PC.</p>
         <p class="muted2">Para cambiar de cuenta, cierra sesión en Claude Code
         (<code>claude /logout</code>) e inicia sesión con otra cuenta.</p>`
      : `<p>Claude Bar uses the local <b>Claude Code</b> session on your PC.</p>
         <p class="muted2">To switch accounts, log out of Claude Code
         (<code>claude /logout</code>) and log in with another account.</p>`;
  openModal(t("logoutTitle"), body);
}

// ----- Ventana / acciones -----
async function setCompact(on: boolean) {
  document.body.classList.toggle("compact", on);
  if (on) {
    await appWindow.setSize(new LogicalSize(COMPACT.w, COMPACT.h));
    await appWindow.setPosition(new LogicalPosition(12, 12));
  } else {
    await appWindow.setSize(new LogicalSize(FULL.w, FULL.h));
  }
}

async function handleAction(act: string) {
  if (act.startsWith("theme:")) {
    setTheme(act.slice(6) as ThemeSetting);
    return;
  }
  if (act.startsWith("provider:")) {
    setProvider(act.slice(9) as Provider);
    return;
  }
  if (act.startsWith("open:")) {
    void openUrl(act.slice(5)).catch((e) => console.error("open:", e));
    return;
  }
  switch (act) {
    case "theme":
      // toggle rápido claro<->oscuro desde la barra de título
      setTheme(document.documentElement.getAttribute("data-theme") === "dark" ? "light" : "dark");
      break;
    case "minimize":
      await appWindow.hide();
      break;
    case "compact":
      await setCompact(true);
      break;
    case "expand":
      await setCompact(false);
      break;
    case "dashboard":
      await openUrl("https://claude.ai/usage");
      break;
    case "status":
      await openUrl("https://status.anthropic.com");
      break;
    case "refresh":
      await invoke("refresh_now");
      break;
    case "refresh-codex":
      await invoke("refresh_codex");
      break;
    case "settings":
      await showSettings();
      break;
    case "about":
      await showAbout();
      break;
    case "logout":
      showLogout();
      break;
    case "lang":
      toggleLang();
      break;
    case "modal-close":
      closeModal();
      break;
    case "quit":
      await invoke("quit");
      break;
  }
}

// ----- Arranque -----
async function main() {
  applyLang();
  applyTheme(loadThemeSetting());

  // El tema "Sistema" debe seguir al SO en vivo.
  window.matchMedia("(prefers-color-scheme: dark)").addEventListener("change", () => {
    if (loadThemeSetting() === "system") applyTheme("system");
  });

  // Delegación: cubre botones presentes y los que se crean dentro del modal.
  document.addEventListener("click", (e) => {
    const el = (e.target as HTMLElement).closest<HTMLElement>("[data-act]");
    if (el) handleAction(el.dataset.act || "");
  });

  // Arrastrar la ventana desde TODO el header, incluso sobre pestañas/botones:
  // un clic (sin mover) activa el botón; si arrastras, mueve la ventana.
  let dragOrigin: { x: number; y: number } | null = null;
  document.querySelector<HTMLElement>(".titlebar")?.addEventListener("mousedown", (e) => {
    if (e.button === 0) dragOrigin = { x: e.clientX, y: e.clientY };
  });
  document.addEventListener("mousemove", (e) => {
    if (!dragOrigin) return;
    const dx = e.clientX - dragOrigin.x;
    const dy = e.clientY - dragOrigin.y;
    if (dx * dx + dy * dy > 16) {
      dragOrigin = null;
      void appWindow.startDragging();
    }
  });
  document.addEventListener("mouseup", () => {
    dragOrigin = null;
  });

  // Sincroniza la bandeja con el proveedor persistido al arrancar.
  void invoke("set_provider", { provider: loadProvider() }).catch(() => {});

  await listen<UsageSnapshot>("usage-updated", (e) => applyUsage(e.payload));
  await listen<CostReport>("cost-updated", (e) => applyCost(e.payload));
  await listen<ProviderStatus>("codex-updated", (e) => applyExternal("codex", e.payload));

  try {
    applyUsage(await invoke<UsageSnapshot>("get_usage"));
    applyCost(await invoke<CostReport>("get_cost"));
  } catch (err) {
    console.error("estado inicial:", err);
  }

  // Reintenta hasta que el primer cálculo de costo esté listo.
  let tries = 0;
  const catchUp = setInterval(async () => {
    tries++;
    try {
      const c = await invoke<CostReport>("get_cost");
      applyCost(c);
      applyUsage(await invoke<UsageSnapshot>("get_usage"));
      if (!c.empty || tries >= 12) clearInterval(catchUp);
    } catch {
      /* reintenta */
    }
  }, 1500);

  setInterval(() => {
    if (loadProvider() === "claude" && lastUpdatedIso && !$("updated").classList.contains("stale")) {
      $("updated").textContent = `${lastPlan} · ${relTime(lastUpdatedIso)}`;
    }
  }, 20_000);
}

main();
