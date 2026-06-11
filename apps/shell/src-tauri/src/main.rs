#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use serde::{Deserialize, Serialize};
use serde_json::Value;
use tauri::{Manager, PhysicalPosition};
use tokio::sync::Mutex;

use rat_client::ManagedClient;

#[derive(Serialize, Deserialize, Clone, Copy)]
struct AvatarPos {
    x: i32,
    y: i32,
}

fn pos_file() -> std::path::PathBuf {
    rat_core::paths::data_dir().join("avatar-pos.json")
}

fn load_pos() -> Option<AvatarPos> {
    serde_json::from_str(&std::fs::read_to_string(pos_file()).ok()?).ok()
}

fn save_pos(p: AvatarPos) {
    let path = pos_file();
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    if let Ok(json) = serde_json::to_string(&p) {
        let _ = std::fs::write(path, json);
    }
}

struct Shell {
    client: Mutex<ManagedClient>,
}

/// Forward one NDJSON-RPC call to ratd. The frontend never opens sockets.
#[tauri::command]
async fn rpc_call(
    state: tauri::State<'_, Shell>,
    method: String,
    params: Option<Value>,
) -> Result<Value, String> {
    let mut client = state.client.lock().await;
    client.call(&method, params.unwrap_or(Value::Null)).await.map_err(|e| e.to_string())
}

#[tauri::command]
async fn open_dashboard(app: tauri::AppHandle) -> Result<(), String> {
    if let Some(w) = app.get_webview_window("dashboard") {
        w.show().map_err(|e| e.to_string())?;
        w.set_focus().map_err(|e| e.to_string())?;
    }
    Ok(())
}

fn main() {
    // GNOME Wayland offers no always-on-top / global positioning to clients;
    // run through XWayland where both work. (spec §13 fallback)
    if std::env::var_os("WAYLAND_DISPLAY").is_some() && std::env::var_os("RAT_NO_X11").is_none() {
        std::env::set_var("GDK_BACKEND", "x11");
    }

    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_env("RAT_LOG")
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    tauri::Builder::default()
        .manage(Shell { client: Mutex::new(ManagedClient::new(rat_core::paths::socket_path())) })
        .invoke_handler(tauri::generate_handler![rpc_call, open_dashboard])
        .setup(|app| {
            // avatar: restore the last dragged position; default = flush bottom-left
            // (torso-up bust — the screen edge is his crop line). spec 2026-06-11
            if let Some(avatar) = app.get_webview_window("avatar") {
                let saved = load_pos().filter(|p| {
                    avatar
                        .available_monitors()
                        .map(|ms| {
                            ms.iter().any(|m| {
                                let mp = m.position();
                                let ms_ = m.size();
                                p.x >= mp.x
                                    && p.x < mp.x + ms_.width as i32
                                    && p.y >= mp.y
                                    && p.y < mp.y + ms_.height as i32
                            })
                        })
                        .unwrap_or(false)
                });
                let pos = saved.or_else(|| {
                    avatar.primary_monitor().ok().flatten().map(|monitor| {
                        let size = monitor.size();
                        let outer = avatar.outer_size().unwrap_or(tauri::PhysicalSize {
                            width: 200,
                            height: 240,
                        });
                        AvatarPos {
                            x: monitor.position().x + 16,
                            y: monitor.position().y + size.height as i32 - outer.height as i32,
                        }
                    })
                });
                if let Some(p) = pos {
                    let _ = avatar.set_position(PhysicalPosition { x: p.x, y: p.y });
                    // mutter ignores pre-map positioning and applies its own placement
                    // once the window maps — re-assert ours shortly after.
                    let av = avatar.clone();
                    std::thread::spawn(move || {
                        for delay_ms in [300u64, 1000] {
                            std::thread::sleep(std::time::Duration::from_millis(delay_ms));
                            let _ = av.set_position(PhysicalPosition { x: p.x, y: p.y });
                        }
                    });
                }
                // persist drags so the next launch reopens where the operator left him;
                // ignore the first 1.5s of Moved events — those are mutter's own
                // placement + our re-assertions, not operator drags.
                let started = std::time::Instant::now();
                avatar.on_window_event(move |event| {
                    if let tauri::WindowEvent::Moved(p) = event {
                        if started.elapsed() > std::time::Duration::from_millis(1500) {
                            save_pos(AvatarPos { x: p.x, y: p.y });
                        }
                    }
                });
            }
            // dashboard closes to tray-less hidden state; avatar lives on
            if let Some(dash) = app.get_webview_window("dashboard") {
                let dash_handle = dash.clone();
                dash.on_window_event(move |event| {
                    if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                        api.prevent_close();
                        let _ = dash_handle.hide();
                    }
                });
            }
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running rato-shell");
}
