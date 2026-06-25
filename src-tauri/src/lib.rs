//! Claude Bar: monitor de uso de Claude para la bandeja de Windows.

mod antigravity;
mod claude_api;
mod codex;
mod cost;
mod credentials;
mod model;
mod pricing;
mod tray_icon;
mod vscdb;

use std::sync::Mutex;
use std::time::{Duration, Instant};

use claude_api::FetchResult;
use model::{CostReport, UsageSnapshot};
use tauri::{
    menu::{CheckMenuItem, Menu, MenuItem, PredefinedMenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    AppHandle, Emitter, Manager,
};
use tauri_plugin_autostart::ManagerExt;
use tauri_plugin_notification::NotificationExt;
use tauri_plugin_opener::OpenerExt;

/// Cada cuanto consultamos el endpoint de uso (limita agresivamente).
const USAGE_INTERVAL_SECS: u64 = 300;
/// Cada cuanto recalculamos el costo desde los logs locales.
const COST_INTERVAL_SECS: u64 = 60;

struct AppState {
    usage: Mutex<UsageSnapshot>,
    cost: Mutex<CostReport>,
    /// Estado previo para detectar transiciones y disparar notificaciones.
    notify: Mutex<NotifyState>,
    /// Última vez que consultamos el endpoint de uso (anti-spam / rate-limit).
    last_usage_fetch: Mutex<Option<Instant>>,
    /// Proveedor seleccionado en la UI; define qué muestra el icono de bandeja.
    provider: Mutex<String>,
}

#[derive(Default)]
struct NotifyState {
    /// false en el primer fetch (no notificamos al arrancar).
    initialized: bool,
    prev_5h_resets_at: Option<String>,
    prev_7d_resets_at: Option<String>,
    /// Para no repetir la notificacion de "limite alcanzado".
    notified_5h_limit: bool,
}

// ----------------------------- Comandos -----------------------------

#[tauri::command]
fn get_usage(state: tauri::State<AppState>) -> UsageSnapshot {
    state.usage.lock().unwrap().clone()
}

#[tauri::command]
fn get_cost(state: tauri::State<AppState>) -> CostReport {
    state.cost.lock().unwrap().clone()
}

#[tauri::command]
fn get_antigravity() -> antigravity::AntigravityStatus {
    antigravity::read()
}

#[tauri::command]
fn get_codex() -> codex::CodexStatus {
    codex::read()
}

#[tauri::command]
fn set_provider(app: AppHandle, provider: String) {
    *app.state::<AppState>().provider.lock().unwrap() = provider;
    refresh_tray_for_provider(&app);
}

#[tauri::command]
fn quit(app: AppHandle) {
    app.exit(0);
}

#[tauri::command]
fn hide_panel(app: AppHandle) {
    if let Some(win) = app.get_webview_window("main") {
        let _ = win.hide();
    }
}

#[tauri::command]
fn refresh_now(app: AppHandle) {
    // Anti-spam: no re-consultar el endpoint si lo hicimos hace <20s
    // (ese endpoint limita muy agresivo y los 429 "se pegan").
    let recent = app
        .state::<AppState>()
        .last_usage_fetch
        .lock()
        .unwrap()
        .map(|t| t.elapsed() < Duration::from_secs(20))
        .unwrap_or(false);
    if !recent {
        let a = app.clone();
        std::thread::spawn(move || {
            fetch_usage_update(&a);
        });
    }
    // El costo es local y barato: siempre se recalcula.
    let b = app.clone();
    std::thread::spawn(move || {
        let report = cost::compute();
        *b.state::<AppState>().cost.lock().unwrap() = report.clone();
        let _ = b.emit("cost-updated", report);
    });
}

// ----------------------------- Ventana -----------------------------

fn position_window(win: &tauri::WebviewWindow, anchor_x: f64, anchor_y: f64) {
    let size = win
        .outer_size()
        .unwrap_or(tauri::PhysicalSize::new(380, 660));
    let w = size.width as f64;
    let h = size.height as f64;
    // Anclamos la esquina inferior-derecha cerca del clic (encima de la barra).
    let mut x = anchor_x - w + 12.0;
    let mut y = anchor_y - h - 12.0;

    if let Ok(Some(mon)) = win.current_monitor() {
        let mp = mon.position();
        let ms = mon.size();
        let left = mp.x as f64;
        let top = mp.y as f64;
        let right = left + ms.width as f64;
        let bottom = top + ms.height as f64;
        if x + w > right {
            x = right - w - 4.0;
        }
        if x < left {
            x = left + 4.0;
        }
        if y + h > bottom {
            y = bottom - h - 4.0;
        }
        if y < top {
            y = top + 4.0;
        }
    } else {
        if x < 0.0 {
            x = anchor_x + 12.0;
        }
        if y < 0.0 {
            y = anchor_y + 12.0;
        }
    }
    let _ = win.set_position(tauri::PhysicalPosition::new(x as i32, y as i32));
}

fn show_window(app: &AppHandle, anchor: Option<(f64, f64)>) {
    if let Some(win) = app.get_webview_window("main") {
        if let Some((x, y)) = anchor {
            position_window(&win, x, y);
        }
        let _ = win.show();
        let _ = win.set_focus();
    }
}

/// Clic en la bandeja: alterna mostrar/ocultar. (Ya NO se auto-oculta al
/// perder foco, asi que la ventana se queda fija y se puede mover libremente.)
fn on_tray_left_click(app: &AppHandle, x: f64, y: f64) {
    if let Some(win) = app.get_webview_window("main") {
        if win.is_visible().unwrap_or(false) {
            let _ = win.hide();
        } else {
            show_window(app, Some((x, y)));
        }
    }
}

// ----------------------------- Refresco -----------------------------

fn tooltip(snap: &UsageSnapshot) -> String {
    if !snap.connected {
        return "Claude Bar — Claude Code no conectado".to_string();
    }
    let reset = if snap.five_hour.resets_in_label.is_empty() {
        String::new()
    } else {
        format!(" · reset {}", snap.five_hour.resets_in_label)
    };
    format!(
        "Claude {} — Sesion {:.0}%{} · Semana {:.0}%",
        snap.plan, snap.five_hour.utilization, reset, snap.seven_day.utilization
    )
}

fn update_tray(app: &AppHandle, percent: Option<f64>, tip: &str) {
    if let Some(tray) = app.tray_by_id("main") {
        let _ = tray.set_icon(Some(tray_icon::render(percent)));
        let _ = tray.set_tooltip(Some(tip));
    }
}

fn update_tray_label(app: &AppHandle, label: &str, bg: [u8; 3], tip: &str) {
    if let Some(tray) = app.tray_by_id("main") {
        let _ = tray.set_icon(Some(tray_icon::render_label(label, bg)));
        let _ = tray.set_tooltip(Some(tip));
    }
}

fn provider_is_claude(app: &AppHandle) -> bool {
    app.state::<AppState>().provider.lock().unwrap().as_str() == "claude"
}

/// Repinta el icono de bandeja segun el proveedor seleccionado. Claude muestra
/// el % de sesion; Codex/Antigravity muestran su inicial (no exponen % local).
fn refresh_tray_for_provider(app: &AppHandle) {
    let provider = app.state::<AppState>().provider.lock().unwrap().clone();
    match provider.as_str() {
        "codex" => {
            let st = codex::read();
            let bg = if st.connected { [16, 163, 127] } else { [120, 120, 130] };
            let tip = if st.connected {
                format!("Codex — {}", st.plan)
            } else {
                "Codex — no conectado".to_string()
            };
            update_tray_label(app, "C", bg, &tip);
        }
        "antigravity" => {
            let st = antigravity::read();
            let bg = if st.connected { [66, 133, 244] } else { [120, 120, 130] };
            let tip = if st.connected {
                format!("Antigravity — {}", st.plan)
            } else {
                "Antigravity — no conectado".to_string()
            };
            update_tray_label(app, "A", bg, &tip);
        }
        _ => {
            let snap = app.state::<AppState>().usage.lock().unwrap().clone();
            let pct = if snap.connected {
                Some(snap.five_hour.utilization)
            } else {
                None
            };
            update_tray(app, pct, &tooltip(&snap));
        }
    }
}

fn send_notification(app: &AppHandle, title: &str, body: &str) {
    let _ = app.notification().builder().title(title).body(body).show();
}

fn parse_ts(s: &Option<String>) -> Option<i64> {
    s.as_ref()
        .and_then(|x| chrono::DateTime::parse_from_rfc3339(x).ok())
        .map(|d| d.timestamp())
}

/// Detecta un reinicio REAL: la ventana saltó hacia adelante > 2 min.
/// (La API devuelve `resets_at` con microsegundos que varían en cada consulta,
/// por eso NO se compara el string exacto: causaría notificaciones cada poll.)
fn reset_happened(prev: &Option<String>, cur: &Option<String>) -> bool {
    match (parse_ts(prev), parse_ts(cur)) {
        (Some(p), Some(c)) => (c - p) > 120,
        (Some(_), None) => true,
        _ => false,
    }
}

/// Detecta transiciones entre snapshots y dispara notificaciones:
///  - reinicio del limite de 5h (cambia el resets_at)
///  - limite de sesion alcanzado (>=95%)
///  - reinicio del limite semanal
fn check_notifications(app: &AppHandle, snap: &UsageSnapshot) {
    if !snap.connected {
        return;
    }
    let state = app.state::<AppState>();
    let mut ns = state.notify.lock().unwrap();
    let cur5 = snap.five_hour.resets_at.clone();
    let cur7 = snap.seven_day.resets_at.clone();

    if ns.initialized {
        // Reinicio de la sesion de 5h (la ventana saltó hacia adelante).
        if reset_happened(&ns.prev_5h_resets_at, &cur5) {
            send_notification(
                app,
                "Sesión de 5h reiniciada",
                "Tu límite de 5 horas se reinició. ¡Listo para seguir!",
            );
            ns.notified_5h_limit = false;
        }
        // Limite de sesion alcanzado.
        if snap.five_hour.utilization >= 95.0 && !ns.notified_5h_limit {
            send_notification(
                app,
                "Límite de sesión (5h)",
                &format!(
                    "Llegaste al {:.0}% de tu sesión de 5 horas.",
                    snap.five_hour.utilization
                ),
            );
            ns.notified_5h_limit = true;
        }
        // Re-arma el aviso cuando el uso baja bastante.
        if snap.five_hour.utilization < 80.0 {
            ns.notified_5h_limit = false;
        }
        // Reinicio del limite semanal.
        if reset_happened(&ns.prev_7d_resets_at, &cur7) {
            send_notification(
                app,
                "Límite semanal reiniciado",
                "Tu límite semanal de Claude se reinició.",
            );
        }
    }

    ns.prev_5h_resets_at = cur5;
    ns.prev_7d_resets_at = cur7;
    ns.initialized = true;
}

enum Outcome {
    Ok,
    RateLimited,
    Error,
}

fn fetch_usage_update(app: &AppHandle) -> Outcome {
    let state = app.state::<AppState>();

    let creds = match credentials::read() {
        Some(c) => c,
        None => {
            let snap = UsageSnapshot {
                connected: false,
                plan: "Claude".to_string(),
                error: Some("not_connected".to_string()),
                updated_at: chrono::Local::now().to_rfc3339(),
                ..Default::default()
            };
            *state.usage.lock().unwrap() = snap.clone();
            if provider_is_claude(app) {
                update_tray(app, None, &tooltip(&snap));
            }
            let _ = app.emit("usage-updated", snap);
            return Outcome::Error;
        }
    };

    *state.last_usage_fetch.lock().unwrap() = Some(Instant::now());
    match claude_api::fetch(&creds) {
        FetchResult::Ok(snap) => {
            eprintln!(
                "[claudebar] usage OK: sesion={:.0}% semana={:.0}% plan={}",
                snap.five_hour.utilization, snap.seven_day.utilization, snap.plan
            );
            *state.usage.lock().unwrap() = snap.clone();
            if provider_is_claude(app) {
                update_tray(app, Some(snap.five_hour.utilization), &tooltip(&snap));
            }
            check_notifications(app, &snap);
            let _ = app.emit("usage-updated", snap);
            Outcome::Ok
        }
        FetchResult::RateLimited => {
            let snap = {
                let mut g = state.usage.lock().unwrap();
                g.connected = true;
                g.plan = creds.plan.clone();
                g.stale = true;
                g.error = Some("rate_limited".to_string());
                g.clone()
            };
            let _ = app.emit("usage-updated", snap);
            Outcome::RateLimited
        }
        FetchResult::Error(e) => {
            let snap = {
                let mut g = state.usage.lock().unwrap();
                g.connected = true;
                g.plan = creds.plan.clone();
                g.stale = true;
                g.error = Some(e);
                g.clone()
            };
            let _ = app.emit("usage-updated", snap.clone());
            // Si nunca tuvimos datos, refleja "sin datos" en el icono.
            let pct = if snap.updated_at.is_empty() {
                None
            } else {
                Some(snap.five_hour.utilization)
            };
            if provider_is_claude(app) {
                update_tray(app, pct, &tooltip(&snap));
            }
            Outcome::Error
        }
    }
}

fn run_usage_loop(app: AppHandle) {
    let mut backoff: u64 = 0;
    loop {
        let outcome = fetch_usage_update(&app);
        let sleep = match outcome {
            Outcome::RateLimited => {
                backoff = (backoff.max(15) * 2).clamp(30, 1800);
                USAGE_INTERVAL_SECS + backoff
            }
            Outcome::Error => {
                backoff = 0;
                // Reintenta antes en errores transitorios de red.
                90
            }
            Outcome::Ok => {
                backoff = 0;
                USAGE_INTERVAL_SECS
            }
        };
        std::thread::sleep(Duration::from_secs(sleep));
    }
}

fn run_cost_loop(app: AppHandle) {
    loop {
        let report = cost::compute();
        eprintln!(
            "[claudebar] cost: hoy=${:.2} ({} tok) · 30d=${:.2}",
            report.today_usd, report.today_tokens, report.last30_usd
        );
        *app.state::<AppState>().cost.lock().unwrap() = report.clone();
        let _ = app.emit("cost-updated", report);
        std::thread::sleep(Duration::from_secs(COST_INTERVAL_SECS));
    }
}

// ----------------------------- Setup -----------------------------

pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            None,
        ))
        .plugin(tauri_plugin_notification::init())
        .manage(AppState {
            usage: Mutex::new(UsageSnapshot::default()),
            cost: Mutex::new(CostReport::default()),
            notify: Mutex::new(NotifyState::default()),
            last_usage_fetch: Mutex::new(None),
            provider: Mutex::new("claude".to_string()),
        })
        .invoke_handler(tauri::generate_handler![
            get_usage,
            get_cost,
            get_antigravity,
            get_codex,
            set_provider,
            refresh_now,
            quit,
            hide_panel
        ])
        .setup(|app| {
            // --- Menu de la bandeja ---
            let open_i = MenuItem::with_id(app, "open", "Abrir Claude Bar", true, None::<&str>)?;
            let dash_i =
                MenuItem::with_id(app, "dashboard", "Usage Dashboard", true, None::<&str>)?;
            let status_i = MenuItem::with_id(app, "status", "Status Page", true, None::<&str>)?;
            // Casilla "Iniciar con Windows" reflejando el estado real.
            let autostart_on = app.autolaunch().is_enabled().unwrap_or(false);
            let autostart_i = CheckMenuItem::with_id(
                app,
                "autostart",
                "Iniciar con Windows",
                true,
                autostart_on,
                None::<&str>,
            )?;
            let quit_i = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;
            let sep1 = PredefinedMenuItem::separator(app)?;
            let sep2 = PredefinedMenuItem::separator(app)?;
            let menu = Menu::with_items(
                app,
                &[&open_i, &sep1, &dash_i, &status_i, &autostart_i, &sep2, &quit_i],
            )?;

            // --- Icono de bandeja ---
            let _tray = TrayIconBuilder::with_id("main")
                .icon(tray_icon::render(None))
                .tooltip("Claude Bar")
                .menu(&menu)
                .show_menu_on_left_click(false)
                .on_menu_event(|app, event| match event.id().as_ref() {
                    "open" => show_window(app, None),
                    "dashboard" => {
                        let _ = app.opener().open_url("https://claude.ai/usage", None::<&str>);
                    }
                    "status" => {
                        let _ = app
                            .opener()
                            .open_url("https://status.anthropic.com", None::<&str>);
                    }
                    "autostart" => {
                        let al = app.autolaunch();
                        if al.is_enabled().unwrap_or(false) {
                            let _ = al.disable();
                        } else {
                            let _ = al.enable();
                        }
                    }
                    "quit" => app.exit(0),
                    _ => {}
                })
                .on_tray_icon_event(|tray, event| {
                    if let TrayIconEvent::Click {
                        button: MouseButton::Left,
                        button_state: MouseButtonState::Up,
                        position,
                        ..
                    } = event
                    {
                        on_tray_left_click(tray.app_handle(), position.x, position.y);
                    }
                })
                .build(app)?;

            // Primer arranque: habilita inicio con Windows y MUESTRA el panel
            // (centrado) para que el usuario vea que funciona sin buscar el icono.
            if let Ok(cfg_dir) = app.path().app_config_dir() {
                let marker = cfg_dir.join(".initialized");
                if !marker.exists() {
                    let _ = app.autolaunch().enable();
                    let _ = std::fs::create_dir_all(&cfg_dir);
                    let _ = std::fs::write(&marker, b"1");
                    eprintln!("[claudebar] primer arranque: autostart on + mostrar panel");
                    if let Some(win) = app.get_webview_window("main") {
                        let _ = win.center();
                        let _ = win.show();
                        let _ = win.set_focus();
                    }
                    send_notification(
                        app.handle(),
                        "Claude Bar activo",
                        "Te avisaré cuando llegues a un límite o se reinicie tu sesión.",
                    );
                }
            }

            eprintln!("[claudebar] tray creado, iniciando hilos de polling");

            // --- Hilos de polling ---
            let h1 = app.handle().clone();
            std::thread::spawn(move || run_usage_loop(h1));
            let h2 = app.handle().clone();
            std::thread::spawn(move || run_cost_loop(h2));

            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("error al iniciar Claude Bar")
        .run(|_app, event| {
            // Mantener la app viva aunque la ventana este oculta.
            if let tauri::RunEvent::ExitRequested { api, .. } = event {
                api.prevent_exit();
            }
        });
}
