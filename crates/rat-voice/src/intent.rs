use regex::Regex;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lang {
    En,
    Pt,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Intent {
    PauseSensors,
    ResumeSensors,
    PrivateOn,
    PrivateOff,
    OpenDashboard,
    PinRecent { minutes: u32 },
    Snooze,
    ModeSwitch { mode: String },
    ApprovalApprove { slug: String },
    ApprovalDeny { slug: String },
    Chat { text: String },
}

#[derive(Debug, Default)]
pub struct IntentRouter;

impl IntentRouter {
    pub fn route(&self, lang: Lang, text: &str) -> Intent {
        let t = normalize(text);

        if any(
            &t,
            &[
                "pause sensors",
                "stop listening",
                "disable sensors",
                "pausar sensores",
                "para sensores",
            ],
        ) {
            return Intent::PauseSensors;
        }
        if any(
            &t,
            &[
                "resume sensors",
                "enable sensors",
                "voltar sensores",
                "retomar sensores",
            ],
        ) {
            return Intent::ResumeSensors;
        }
        if any(
            &t,
            &[
                "private mode",
                "go private",
                "modo privado",
                "privado ligado",
            ],
        ) {
            return Intent::PrivateOn;
        }
        if any(
            &t,
            &[
                "exit private",
                "private off",
                "sair do privado",
                "privado desligado",
            ],
        ) {
            return Intent::PrivateOff;
        }
        if any(
            &t,
            &[
                "open dashboard",
                "show dashboard",
                "abrir painel",
                "mostrar painel",
            ],
        ) {
            return Intent::OpenDashboard;
        }
        if any(&t, &["pin that", "pin this", "pina isso", "fixa isso"]) {
            return Intent::PinRecent { minutes: 2 };
        }
        if any(&t, &["snooze", "remind me later", "adiar", "soneca"]) {
            return Intent::Snooze;
        }

        if let Some(mode) = capture_mode(&t, lang) {
            return Intent::ModeSwitch { mode };
        }
        if let Some(slug) = capture_slug(&t, &["approve", "aprovar", "aprova"]) {
            return Intent::ApprovalApprove { slug };
        }
        if let Some(slug) = capture_slug(&t, &["deny", "reject", "negar", "rejeitar", "nega"]) {
            return Intent::ApprovalDeny { slug };
        }

        Intent::Chat {
            text: text.trim().to_string(),
        }
    }
}

fn normalize(text: &str) -> String {
    text.to_lowercase()
        .replace(['á', 'à', 'ã', 'â'], "a")
        .replace(['é', 'ê'], "e")
        .replace('í', "i")
        .replace(['ó', 'ô', 'õ'], "o")
        .replace('ú', "u")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn any(text: &str, needles: &[&str]) -> bool {
    needles.iter().any(|needle| text.contains(needle))
}

fn capture_mode(text: &str, _lang: Lang) -> Option<String> {
    for mode in ["mentor", "chaos", "quiet", "hype", "rubber duck"] {
        if text.contains(&format!("mode {mode}")) || text.contains(&format!("modo {mode}")) {
            return Some(mode.replace(' ', "-"));
        }
    }
    None
}

fn capture_slug(text: &str, verbs: &[&str]) -> Option<String> {
    for verb in verbs {
        let pattern = format!(r"\b{}\s+([a-z]+-[a-z]+)\b", regex::escape(verb));
        let re = Regex::new(&pattern).ok()?;
        if let Some(caps) = re.captures(text) {
            return caps.get(1).map(|m| m.as_str().to_string());
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn routes_english_control_intents() {
        let r = IntentRouter;
        assert_eq!(r.route(Lang::En, "pause sensors"), Intent::PauseSensors);
        assert_eq!(r.route(Lang::En, "resume sensors"), Intent::ResumeSensors);
        assert_eq!(r.route(Lang::En, "go private"), Intent::PrivateOn);
        assert_eq!(r.route(Lang::En, "exit private"), Intent::PrivateOff);
        assert_eq!(r.route(Lang::En, "open dashboard"), Intent::OpenDashboard);
        assert_eq!(
            r.route(Lang::En, "pin that"),
            Intent::PinRecent { minutes: 2 }
        );
        assert_eq!(r.route(Lang::En, "snooze"), Intent::Snooze);
    }

    #[test]
    fn routes_portuguese_control_intents() {
        let r = IntentRouter;
        assert_eq!(r.route(Lang::Pt, "pausar sensores"), Intent::PauseSensors);
        assert_eq!(r.route(Lang::Pt, "retomar sensores"), Intent::ResumeSensors);
        assert_eq!(r.route(Lang::Pt, "modo privado"), Intent::PrivateOn);
        assert_eq!(r.route(Lang::Pt, "sair do privado"), Intent::PrivateOff);
        assert_eq!(r.route(Lang::Pt, "abrir painel"), Intent::OpenDashboard);
        assert_eq!(
            r.route(Lang::Pt, "pina isso"),
            Intent::PinRecent { minutes: 2 }
        );
    }

    #[test]
    fn routes_approval_slugs() {
        let r = IntentRouter;
        assert_eq!(
            r.route(Lang::En, "approve amber-pixel"),
            Intent::ApprovalApprove {
                slug: "amber-pixel".into()
            }
        );
        assert_eq!(
            r.route(Lang::Pt, "negar brisk-anchor"),
            Intent::ApprovalDeny {
                slug: "brisk-anchor".into()
            }
        );
    }

    #[test]
    fn non_match_is_chat() {
        let r = IntentRouter;
        assert_eq!(
            r.route(Lang::En, "what is going on with these tests"),
            Intent::Chat {
                text: "what is going on with these tests".into()
            }
        );
    }
}
