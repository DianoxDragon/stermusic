//! Frame pacing. Vertical sync is off (see `main.rs`): without it, a continuous animation
//! (visualizer) would draw thousands of frames per second. Each frame therefore waits its turn,
//! at the refresh rate of the screen the window is on.

use std::collections::HashMap;
use std::process::{Command, Stdio};
use std::sync::OnceLock;
use std::time::{Duration, Instant};

/// Refresh rate used when the screen's is unknown.
const DEFAULT_HZ: f32 = 60.0;

/// Refresh rates of the screens (by name: "DP-1"…), read once in the background.
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
    // KDE Plasma: kscreen-doctor gives the mode of each screen.
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
    // Elsewhere: xrandr (Xwayland), current mode marked with a star.
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

/// Refresh rate of the `output` screen (otherwise the highest of the known screens).
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
    /// Waits, if needed, until one frame interval (at the refresh rate of the `output` screen) has
    /// elapsed since the previous frame. Minimized or hidden window (`unseen`): four frames per
    /// second at most, enough for playback to keep going (next track, sleep timer…).
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
