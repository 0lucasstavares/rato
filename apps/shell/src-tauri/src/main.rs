#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use serde_json::Value;
use tauri::{Manager, PhysicalPosition};
use tokio::sync::Mutex;

use rat_client::ManagedClient;

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
            // avatar: bottom-left of the primary monitor, with margins
            if let Some(avatar) = app.get_webview_window("avatar") {
                if let Ok(Some(monitor)) = avatar.primary_monitor() {
                    let size = monitor.size();
                    let outer = avatar.outer_size().unwrap_or(tauri::PhysicalSize {
                        width: 320,
                        height: 380,
                    });
                    let margin = 16;
                    let x = monitor.position().x + margin;
                    let y = monitor.position().y + size.height as i32 - outer.height as i32 - margin - 48;
                    let _ = avatar.set_position(PhysicalPosition { x, y });
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
            }
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running rato-shell");
}
