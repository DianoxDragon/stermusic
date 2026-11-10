//! Cadence des images. La synchronisation verticale est coupée (voir `main.rs`) : sans elle,
//! une animation continue (visualiseur) dessinerait des milliers d'images par seconde. Chaque
//! image attend donc son tour, au rythme de l'écran où se trouve la fenêtre.

use std::collections::HashMap;
use std::process::{Command, Stdio};
use std::sync::OnceLock;
use std::time::{Duration, Instant};

/// Fréquence prise quand celle de l'écran est inconnue.
const DEFAULT_HZ: f32 = 60.0;

/// Fréquences des écrans (par nom : « DP-1 »…), lues une fois en arrière-plan.
fn rates() -> Option<&'static HashMap<String, f32>> {
    static RATES: OnceLock<HashMap<String, f32>> = OnceLock::new();
    static STARTED: std::sync::Once = std::sync::Once::new();
    STARTED.call_once(|| {
        std::thread::spawn(|| {
            let _ = RATES.set(read_rates());
        });
    });
    RATES.get()
}

fn read_rates() -> HashMap<String, f32> {
    let mut out = HashMap::new();
    // KDE Plasma : kscreen-doctor donne le mode de chaque écran.
    if let Ok(o) = Command::new("kscreen-doctor").arg("-j").stderr(Stdio::null()).output() {
        if let Ok(v) = serde_json::from_slice::<serde_json::Value>(&o.stdout) {
            for output in v.get("outputs").and_then(|o| o.as_array()).into_iter().flatten() {
                let current = output.get("currentModeId");
                let mode = output
                    .get("modes")
                    .and_then(|m| m.as_array())
                    .and_then(|m| m.iter().find(|m| m.get("id") == current));
                if let (Some(name), Some(hz)) = (
                    output.get("name").and_then(|n| n.as_str()),
                    mode.and_then(|m| m.get("refreshRate")).and_then(|r| r.as_f64()),
                ) {
                    out.insert(name.to_owned(), hz as f32);
                }
            }
        }
    }
    // Ailleurs : xrandr (Xwayland), mode courant marqué d'une étoile.
    if out.is_empty() {
        if let Ok(o) = Command::new("xrandr").arg("--current").stderr(Stdio::null()).output() {
            let text = String::from_utf8_lossy(&o.stdout);
            let mut name = String::new();
            for line in text.lines() {
                if !line.starts_with(' ') {
                    name = line.split_whitespace().next().unwrap_or_default().to_owned();
                } else if let Some(hz) = line
                    .split_whitespace()
                    .find(|w| w.contains('*'))
                    .and_then(|w| w.trim_end_matches(['*', '+']).parse::<f32>().ok())
                {
                    out.insert(name.clone(), hz);
                }
            }
        }
    }
    out
}

/// Fréquence de l'écran `output` (sinon la plus haute des écrans connus).
fn refresh_rate(output: Option<&str>) -> f32 {
    let Some(rates) = rates() else { return DEFAULT_HZ };
    output
        .and_then(|o| rates.get(o).copied())
        .or_else(|| rates.values().copied().reduce(f32::max))
        .filter(|hz| (20.0..=500.0).contains(hz))
        .unwrap_or(DEFAULT_HZ)
}

#[derive(Default)]
pub struct Pacer {
    last: Option<Instant>,
}

impl Pacer {
    /// Attend, si besoin, que l'intervalle d'une image (à la fréquence de l'écran `output`) soit
    /// écoulé depuis la précédente. Fenêtre réduite ou cachée (`unseen`) : quatre images par
    /// seconde au plus, assez pour que la lecture suive (morceau suivant, minuterie…).
    pub fn wait(&mut self, output: Option<&str>, unseen: bool) {
        let hz = if unseen { 4.0 } else { refresh_rate(output) };
        let interval = Duration::from_secs_f32(1.0 / hz);
        if let Some(last) = self.last {
            let since = last.elapsed();
            if since < interval {
                std::thread::sleep(interval - since);
            }
        }
        self.last = Some(Instant::now());
    }
}
