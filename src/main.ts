import "./styles.css";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { getVersion } from "@tauri-apps/api/app";
import { openUrl } from "@tauri-apps/plugin-opener";
import { getCurrentWindow, Effect, EffectState } from "@tauri-apps/api/window";
import { LogicalSize, LogicalPosition, PhysicalPosition } from "@tauri-apps/api/dpi";

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
    connect: "Sin conexión a Claude Code · tocá para saber por qué", langBtn: "English",
    aboutTitle: "Acerca de Claude Bar", settingsTitle: "Ajustes", logoutTitle: "Cerrar sesión (Claude)",
    theme: "Tema", whyTitle: "¿Por qué no conecta?",
    background: "Fondo", choosePhoto: "Elegir foto…", removePhoto: "Quitar", transparency: "Transparencia", blur: "Desenfoque",
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
    connect: "Not connected to Claude Code · tap to learn why", langBtn: "Español",
    aboutTitle: "About Claude Bar", settingsTitle: "Settings", logoutTitle: "Log out (Claude)",
    theme: "Theme", whyTitle: "Why not connected?",
    background: "Background", choosePhoto: "Choose photo…", removePhoto: "Remove", transparency: "Transparency", blur: "Blur",
  },
};
let lang = localStorage.getItem("lang") === "en" ? "en" : "es";
const t = (k: string) => I18N[lang][k] ?? k;

const $ = (id: string) => document.getElementById(id)!;
const appWindow = getCurrentWindow();
const FULL = { w: 384, h: 700 };
const COMPACT = { w: 228, h: 112 };

// ----- Tema -----
type Theme = "auto" | "midnight" | "daylight";
let theme: Theme = ((): Theme => {
  const s = localStorage.getItem("theme");
  return s === "daylight" || s === "auto" || s === "midnight" ? s : "midnight";
})();

function resolveAuto(): "midnight" | "daylight" {
  return window.matchMedia("(prefers-color-scheme: dark)").matches ? "midnight" : "daylight";
}

async function applyWindowEffect(th: Theme) {
  try {
    if (th === "daylight") {
      await appWindow.setEffects({
        effects: [Effect.Acrylic], state: EffectState.Active, radius: 14,
        color: [245, 247, 251, 205],
      });
    } else if (th === "midnight") {
      await appWindow.setEffects({
        effects: [Effect.Acrylic], state: EffectState.Active, radius: 14,
        color: [24, 26, 36, 210],
      });
    } else {
      // Auto: Mica sigue el tema del sistema (Windows 11).
      await appWindow.setEffects({ effects: [Effect.Mica], state: EffectState.Active, radius: 14 });
    }
  } catch (e) {
    console.error("setEffects:", e);
  }
}

async function applyTheme(th: Theme) {
  theme = th;
  localStorage.setItem("theme", th);
  const eff = th === "auto" ? resolveAuto() : th;
  document.documentElement.setAttribute("data-theme", eff);
  applyPanelSurface();
  await applyWindowEffect(th);
}

// ----- Fondo custom (foto + transparencia) -----
const THEME_BASE: Record<string, string> = {
  midnight: "28,30,42",
  daylight: "248,250,253",
};

/// Pinta el "scrim" del panel: tinte del tema activo con la opacidad elegida,
/// para que la foto de fondo se vea translúcida y el texto siga legible.
function applyPanelSurface() {
  const eff = theme === "auto" ? resolveAuto() : theme;
  const base = THEME_BASE[eff] || THEME_BASE.midnight;
  const op = parseFloat(localStorage.getItem("bgOpacity") || "0.45");
  ($("panel") as HTMLElement).style.background = `rgba(${base},${op})`;
}

function getGallery(): string[] {
  try {
    const raw = localStorage.getItem("bgGallery");
    return raw ? (JSON.parse(raw) as string[]) : [];
  } catch {
    return [];
  }
}
function saveGallery(arr: string[]) {
  localStorage.setItem("bgGallery", JSON.stringify(arr));
}

function applyBg() {
  const stored = localStorage.getItem("bgPhoto");
  const photo = stored && stored.length > 0 ? stored : null;
  const el = $("bg-photo") as HTMLElement;
  const panel = $("panel") as HTMLElement;
  if (photo) {
    el.style.backgroundImage = `url("${photo}")`;
    document.body.classList.add("has-bg");
    const b = parseInt(localStorage.getItem("bgBlur") || "13", 10);
    panel.style.backdropFilter = `blur(${b}px)`;
    (panel.style as { webkitBackdropFilter?: string }).webkitBackdropFilter = `blur(${b}px)`;
  } else {
    el.style.backgroundImage = "";
    document.body.classList.remove("has-bg");
    panel.style.backdropFilter = "";
    (panel.style as { webkitBackdropFilter?: string }).webkitBackdropFilter = "";
  }
  applyPanelSurface();
}

function useNoBg() {
  localStorage.removeItem("bgPhoto");
  applyBg();
}
function useUserBg(dataUrl: string) {
  localStorage.setItem("bgPhoto", dataUrl);
  applyBg();
}

/// Reescala la imagen elegida para que entre cómoda en localStorage.
function fileToScaledDataUrl(file: File, maxDim: number, quality: number): Promise<string> {
  return new Promise((resolve, reject) => {
    const img = new Image();
    const url = URL.createObjectURL(file);
    img.onload = () => {
      URL.revokeObjectURL(url);
      const scale = Math.min(1, maxDim / Math.max(img.width, img.height));
      const w = Math.round(img.width * scale);
      const h = Math.round(img.height * scale);
      const canvas = document.createElement("canvas");
      canvas.width = w;
      canvas.height = h;
      const ctx = canvas.getContext("2d");
      if (!ctx) return reject(new Error("no 2d ctx"));
      ctx.drawImage(img, 0, 0, w, h);
      resolve(canvas.toDataURL("image/jpeg", quality));
    };
    img.onerror = () => reject(new Error("img load error"));
    img.src = url;
  });
}

async function handleBgFile(file: File) {
  // Intenta a buena calidad; si no entra en localStorage, baja resolución.
  for (const [dim, q] of [[1200, 0.82], [900, 0.78], [640, 0.7]] as [number, number][]) {
    try {
      const dataUrl = await fileToScaledDataUrl(file, dim, q);
      const gallery = getGallery();
      if (!gallery.includes(dataUrl)) {
        gallery.push(dataUrl);
        while (gallery.length > 6) gallery.shift();
        try {
          saveGallery(gallery);
        } catch {
          // localStorage lleno: descarta la más vieja y reintenta.
          gallery.shift();
          saveGallery(gallery);
        }
      }
      useUserBg(dataUrl);
      renderGallery();
      return;
    } catch (e) {
      if (dim === 640) console.error("bg photo:", e);
    }
  }
}

function removeFromGallery(dataUrl: string) {
  saveGallery(getGallery().filter((p) => p !== dataUrl));
  if (localStorage.getItem("bgPhoto") === dataUrl) useNoBg();
  renderGallery();
}

/// Dibuja las miniaturas de la galería en Ajustes (si el modal está abierto).
function renderGallery() {
  const box = document.getElementById("bg-gallery");
  if (!box) return;
  const stored = localStorage.getItem("bgPhoto");
  box.innerHTML = "";

  // tile: sin fondo (estado por defecto)
  const none = document.createElement("button");
  none.className = "thumb none" + (!stored ? " active" : "");
  none.title = lang === "es" ? "Sin fondo" : "No background";
  none.textContent = "∅";
  none.addEventListener("click", () => {
    useNoBg();
    renderGallery();
  });
  box.appendChild(none);

  for (const p of getGallery()) {
    const wrap = document.createElement("div");
    wrap.className = "thumb-wrap";
    const th = document.createElement("button");
    th.className = "thumb" + (p === stored ? " active" : "");
    th.style.backgroundImage = `url("${p}")`;
    th.addEventListener("click", () => {
      useUserBg(p);
      renderGallery();
    });
    const x = document.createElement("button");
    x.className = "thumb-x";
    x.textContent = "✕";
    x.addEventListener("click", (e) => {
      e.stopPropagation();
      removeFromGallery(p);
    });
    wrap.append(th, x);
    box.appendChild(wrap);
  }

  const add = document.createElement("button");
  add.className = "thumb add";
  add.title = lang === "es" ? "Agregar foto" : "Add photo";
  add.textContent = "+";
  add.addEventListener("click", () => ($("bg-file") as HTMLInputElement).click());
  box.appendChild(add);
}

// ----- Auto-ajuste de alto de la ventana al contenido -----
let lastFitH = FULL.h;
let fitScheduled = false;
function scheduleFit() {
  if (fitScheduled) return;
  fitScheduled = true;
  requestAnimationFrame(() => {
    fitScheduled = false;
    fitWindowHeight();
  });
}
async function fitWindowHeight() {
  if (document.body.classList.contains("compact")) return;
  const fv = $("full-view").getBoundingClientRect().height;
  const tb = (document.querySelector(".titlebar") as HTMLElement).getBoundingClientRect().height;
  const target = Math.round(Math.min(700, Math.max(200, fv + tb + 28)));
  if (Math.abs(target - lastFitH) < 4) return;
  try {
    const sf = await appWindow.scaleFactor();
    const pos = await appWindow.outerPosition();
    const size = await appWindow.outerSize();
    // Mantiene fija la base (queda anclada sobre la bandeja) al cambiar de alto.
    const deltaLogical = size.height / sf - target;
    const newY = Math.round(pos.y + deltaLogical * sf);
    await appWindow.setSize(new LogicalSize(FULL.w, target));
    await appWindow.setPosition(new PhysicalPosition(pos.x, newY));
    lastFitH = target;
  } catch (e) {
    console.error("fit:", e);
  }
}

/// Clase de severidad para la barra (espeja los umbrales del icono de bandeja).
function sevClass(util: number): string {
  if (util >= 90) return "sev3";
  if (util >= 70) return "sev2";
  if (util >= 40) return "sev1";
  return "";
}

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
  const el = $(id) as HTMLElement;
  el.style.width = Math.max(0, Math.min(100, util)) + "%";
  el.classList.remove("sev1", "sev2", "sev3");
  const c = sevClass(util);
  if (c) el.classList.add(c);
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
  $("plan-badge").textContent = lastPlan;

  document.body.classList.toggle("disconnected", !u.connected);

  const updated = $("updated");
  if (!u.connected) {
    updated.textContent = t("connect");
    updated.classList.add("stale");
  } else if (u.error && u.stale) {
    updated.textContent = `${u.plan} · ${u.error}`;
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

  scheduleFit();
}

function applyCost(c: CostReport) {
  const prev = lastCost;
  lastCost = c;
  if (prev && prev.todayUsd !== c.todayUsd) {
    const el = $("cost-today-val");
    el.classList.remove("bump");
    void el.offsetWidth; // fuerza reflow para reiniciar la animación
    el.classList.add("bump");
  }
  $("cost-today-val").textContent = fmtUsd(c.todayUsd);
  $("cost-today-sub").textContent = `${t("today")} · ${fmtTokens(c.todayTokens)} ${t("tokens")}`;
  $("cost-week").textContent = `${t("week")}: ${fmtUsd(c.weekUsd)}`;
  $("cost-30").textContent = `${t("last30")}: ${fmtUsd(c.last30Usd)} · ${fmtTokens(c.last30Tokens)} ${t("tokens")}`;
  $("cost-note").textContent = t("costNote");

  scheduleFit();
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
  const themeBlock = `<div class="set-label">${t("theme")}</div>
    <div class="seg" id="theme-seg">
      <button data-theme-opt="auto">Auto</button>
      <button data-theme-opt="midnight">Midnight</button>
      <button data-theme-opt="daylight">Daylight</button>
    </div>`;
  const opPct = Math.round(parseFloat(localStorage.getItem("bgOpacity") || "0.45") * 100);
  const blurPx = parseInt(localStorage.getItem("bgBlur") || "13", 10);
  const bgBlock = `<div class="set-label">${t("background")}</div>
    <div class="gallery" id="bg-gallery"></div>
    <div class="set-label">${t("transparency")}</div>
    <input type="range" id="bg-op" class="range" min="5" max="85" value="${opPct}" />
    <div class="set-label">${t("blur")}</div>
    <input type="range" id="bg-blur" class="range" min="0" max="28" value="${blurPx}" />`;
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
  openModal(t("settingsTitle"), themeBlock + bgBlock + rows);

  document.querySelectorAll<HTMLButtonElement>("#theme-seg [data-theme-opt]").forEach((b) => {
    if (b.dataset.themeOpt === theme) b.classList.add("active");
    b.addEventListener("click", async () => {
      document
        .querySelectorAll("#theme-seg [data-theme-opt]")
        .forEach((x) => x.classList.remove("active"));
      b.classList.add("active");
      await applyTheme(b.dataset.themeOpt as Theme);
    });
  });

  renderGallery();
  $("bg-op").addEventListener("input", (e) => {
    const v = parseInt((e.target as HTMLInputElement).value, 10) / 100;
    localStorage.setItem("bgOpacity", String(v));
    applyPanelSurface();
  });
  $("bg-blur").addEventListener("input", (e) => {
    localStorage.setItem("bgBlur", (e.target as HTMLInputElement).value);
    applyBg();
  });
}

function showWhyDisconnected() {
  const body =
    lang === "es"
      ? `<p>Claude Bar lee tu token de <b>Claude Code</b> desde
           <code>~/.claude/.credentials.json</code> para mostrar los límites en vivo.</p>
         <p class="muted2">En esta PC ese archivo no existe: tu Claude Code guarda la sesión
           de forma segura (llavero del SO) o la provee el entorno, así que no hay token en
           texto plano para leer. Por eso Sesión / Semanal quedan en 0%.</p>
         <p class="muted2"><b>Costo</b> sí funciona: se calcula desde los registros locales de
           uso, sin necesitar el token.</p>`
      : `<p>Claude Bar reads your <b>Claude Code</b> token from
           <code>~/.claude/.credentials.json</code> to show live limits.</p>
         <p class="muted2">On this PC that file doesn't exist: your Claude Code stores the
           session securely (OS keychain) or it's provided by the environment, so there's no
           plaintext token to read. That's why Session / Weekly stay at 0%.</p>
         <p class="muted2"><b>Cost</b> still works: it's computed from local usage logs, no
           token needed.</p>`;
  openModal(t("whyTitle"), body);
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
    lastFitH = FULL.h;
    scheduleFit();
  }
}

async function handleAction(act: string) {
  switch (act) {
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
  await applyTheme(theme);
  applyBg();

  // Carga la foto de fondo cuando el usuario la elige.
  ($("bg-file") as HTMLInputElement).addEventListener("change", (e) => {
    const f = (e.target as HTMLInputElement).files?.[0];
    if (f) handleBgFile(f);
  });

  // El modo Auto sigue el tema del sistema en tiempo real.
  window.matchMedia("(prefers-color-scheme: dark)").addEventListener("change", () => {
    if (theme === "auto") applyTheme("auto");
  });

  document.querySelectorAll<HTMLButtonElement>("[data-act]").forEach((btn) => {
    btn.addEventListener("click", () => handleAction(btn.dataset.act || ""));
  });

  // Al tocar el estado "no conectado", explica por qué.
  $("updated").addEventListener("click", () => {
    if (document.body.classList.contains("disconnected")) showWhyDisconnected();
  });

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
