use zbus::Connection;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Backend {
    Mutter,
    ScreenSaver,
    None,
}

/// Desktop idle-time probe. GNOME Mutter IdleMonitor first, then the
/// freedesktop ScreenSaver interface (KDE etc.), else None — the daemon falls
/// back to last-sensor-event time.
pub struct IdleProbe {
    conn: Option<Connection>,
    backend: Backend,
}

impl IdleProbe {
    pub async fn connect() -> Self {
        let Ok(conn) = Connection::session().await else {
            tracing::warn!("no session D-Bus; idle detection falls back to sensor activity");
            return Self {
                conn: None,
                backend: Backend::None,
            };
        };
        let probe = Self {
            conn: Some(conn),
            backend: Backend::Mutter,
        };
        if probe.query(Backend::Mutter).await.is_some() {
            tracing::info!("idle probe: GNOME Mutter IdleMonitor");
            return Self {
                backend: Backend::Mutter,
                ..probe
            };
        }
        if probe.query(Backend::ScreenSaver).await.is_some() {
            tracing::info!("idle probe: org.freedesktop.ScreenSaver");
            return Self {
                backend: Backend::ScreenSaver,
                ..probe
            };
        }
        tracing::warn!("no D-Bus idle interface; idle detection falls back to sensor activity");
        Self {
            backend: Backend::None,
            ..probe
        }
    }

    pub async fn idle_ms(&self) -> Option<i64> {
        self.query(self.backend).await
    }

    async fn query(&self, backend: Backend) -> Option<i64> {
        let conn = self.conn.as_ref()?;
        match backend {
            Backend::Mutter => {
                let reply = conn
                    .call_method(
                        Some("org.gnome.Mutter.IdleMonitor"),
                        "/org/gnome/Mutter/IdleMonitor/Core",
                        Some("org.gnome.Mutter.IdleMonitor"),
                        "GetIdletime",
                        &(),
                    )
                    .await
                    .ok()?;
                let ms: u64 = reply.body().deserialize().ok()?;
                Some(ms as i64)
            }
            Backend::ScreenSaver => {
                let reply = conn
                    .call_method(
                        Some("org.freedesktop.ScreenSaver"),
                        "/org/freedesktop/ScreenSaver",
                        Some("org.freedesktop.ScreenSaver"),
                        "GetSessionIdleTime",
                        &(),
                    )
                    .await
                    .ok()?;
                let secs: u32 = reply.body().deserialize().ok()?;
                Some(i64::from(secs) * 1000)
            }
            Backend::None => None,
        }
    }
}
