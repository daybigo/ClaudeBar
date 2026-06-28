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
  extraUsage: ExtraUsage;
  stale: boolean;
  error: string | null;
  updatedAt: string;
}
interface CostReport {
  todayUsd: number;
  todayTokens: number;
  weekUsd: number;
  monthUsd: number;
  last30Usd: number;
  last30Tokens: number;
  updatedAt: string;
  empty: boolean;
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
let lang = localStorage.getItem("lang") === "en" ? "en" : "es";
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
}
function bucketLabel(b: AntigravityBucket): string {
  if (b.window === "5h") return t("session");
  if (b.window === "weekly") return t("weekly");
  return b.label || b.window;
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
  return `<div class="ubars">${groups
    .map(
      (g) =>
        `<div class="ugroup">${esc(g.name)}</div>${g.items
          .map((b) => {
            const pct = Math.max(0, Math.min(100, b.usedPercent));
            const reset = resetLabel(b.resetsAt);
            return `<div class="ublock">
        <div class="urow"><span class="ulabel sub">${bucketLabel(b)}</span><span class="ureset">${reset ? `${t("resetsIn")} ${reset}` : ""}</span></div>
        <div class="bar"><div class="fill" style="width:${pct}%"></div></div>
        <div class="upct">${fmtPct(b.usedPercent)} ${t("used")}</div>
      </div>`;
          })
          .join("")}`
    )
    .join("")}</div>`;
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
function usageBars(st: ProviderStatus): string {
  const wins = [st.primary, st.secondary].filter(Boolean) as UsageWindow[];
  if (!wins.length) return "";
  return `<div class="ubars">${wins
    .map((w) => {
      const pct = Math.max(0, Math.min(100, w.usedPercent));
      const reset = resetLabel(w.resetsAt);
      return `<div class="ublock">
        <div class="urow"><span class="ulabel">${windowLabel(w.windowMinutes)}</span><span class="ureset">${reset ? `${t("resetsIn")} ${reset}` : ""}</span></div>
        <div class="bar"><div class="fill" style="width:${pct}%"></div></div>
        <div class="upct">${fmtPct(w.usedPercent)} ${t("used")}</div>
      </div>`;
    })
    .join("")}</div>`;
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
  return `<div class="pcard">
      <div class="pemblem" style="background:${PROVIDER_COLOR[p]}">${initial}</div>
      <div class="pplan">${esc(st.plan)}</div>
      ${st.email ? `<div class="pemail">${esc(st.email)}</div>` : ""}
      ${bars}
      ${url ? `<button class="pcard-btn" data-act="open:${url}">${t("open")} ${esc(label)} ↗</button>` : ""}
      ${bars ? "" : `<p class="pnote">${p === "antigravity" ? t("openAntigravity") : t("usageNotHere")}</p>`}
    </div>`;
}

async function refreshExternal(p: Provider, command: string): Promise<void> {
  let st: ProviderStatus = { connected: false, email: "", plan: "" };
  try {
    st = await invoke<ProviderStatus>(command);
  } catch (e) {
    console.error(command, e);
  }
  if (loadProvider() !== p) return; // el usuario cambió mientras tanto
  $("plan-badge").textContent = st.connected ? st.plan : "";
  const updated = $("updated");
  updated.classList.remove("stale");
  updated.textContent = st.connected ? t("connected") : t("notConnected");
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
function applyUsage(u: UsageSnapshot) {
  lastUsage = u;
  lastPlan = u.connected ? u.plan : "";
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

  // Sonnet
  const sonnet = u.sevenDaySonnet;
  if (sonnet) {
    setBar("sonnet-fill", sonnet.utilization);
    $("sonnet-pct").textContent = `${fmtPct(sonnet.utilization)} ${t("used")}`;
    $("sonnet-block").style.display = "";
  } else {
    $("sonnet-block").style.display = "none";
  }

  // Uso extra
  const ex = u.extraUsage;
  setBar("extra-fill", ex.utilization);
  $("extra-amount").textContent = `${t("thisMonth")}: ${fmtUsd(ex.usedUsd)} / ${fmtUsd(ex.limitUsd)}`;
  $("extra-pct").textContent = `${fmtPct(ex.utilization)} ${t("used")}`;
}

function applyCost(c: CostReport) {
  lastCost = c;
  $("cost-today").textContent = `${t("today")}: ${fmtUsd(c.todayUsd)} · ${fmtTokens(c.todayTokens)} ${t("tokens")}`;
  $("cost-week").textContent = `${t("week")}: ${fmtUsd(c.weekUsd)}`;
  $("cost-30").textContent = `${t("last30")}: ${fmtUsd(c.last30Usd)} · ${fmtTokens(c.last30Tokens)} ${t("tokens")}`;
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

  // Sincroniza la bandeja con el proveedor persistido al arrancar.
  void invoke("set_provider", { provider: loadProvider() }).catch(() => {});

  await listen<UsageSnapshot>("usage-updated", (e) => applyUsage(e.payload));
  await listen<CostReport>("cost-updated", (e) => applyCost(e.payload));

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
    if (lastUpdatedIso && !$("updated").classList.contains("stale")) {
      $("updated").textContent = `${lastPlan} · ${relTime(lastUpdatedIso)}`;
    }
  }, 20_000);
}

main();
