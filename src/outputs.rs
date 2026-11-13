//! Audio output selection (speakers, USB headset, HDMI…).
//!
//! Sound goes through PipeWire's ALSA plugin (rodio's "default" output). This plugin reads the
//! `PIPEWIRE_NODE` variable each time the output is opened: setting it before opening the
//! stream is enough to send it to the chosen output, without bypassing PipeWire (mixing,
//! Plasma's volume and the other applications keep working). Without PipeWire, `PULSE_SINK`
//! does the same for PulseAudio.
//!
//! The list of outputs comes from `pactl` (pipewire-pulse), which gives their readable name.

use serde::Deserialize;

#[derive(Clone, Debug, PartialEq)]
pub struct Output {
    /// Internal name (`alsa_output.usb-…`), the one PipeWire expects.
    pub name: String,
    /// Readable name ("Yeti Stereo Headset").
    pub description: String,
}

#[derive(Deserialize)]
struct Sink {
    name: String,
    #[serde(default)]
    description: String,
}

/// The system's audio outputs, in `pactl` order; empty if `pactl` is missing.
pub fn list() -> Vec<Output> {
    let Ok(out) = std::process::Command::new("pactl")
        .args(["-f", "json", "list", "sinks"])
        .output()
    else {
        return Vec::new();
    };
    parse(&out.stdout)
}

fn parse(json: &[u8]) -> Vec<Output> {
    serde_json::from_slice::<Vec<Sink>>(json)
        .unwrap_or_default()
        .into_iter()
        .map(|s| Output {
            description: if s.description.trim().is_empty() {
                s.name.clone()
            } else {
                s.description.trim().to_owned()
            },
            name: s.name,
        })
        .collect()
}

/// Points the next openings of the audio output at `name` (empty: default output).
///
/// Only called when the choice changes (at startup, or from the settings): the environment
/// variable is read by the ALSA plugin when the stream is opened.
pub fn route_to(name: &str) {
    for var in ["PIPEWIRE_NODE", "PULSE_SINK"] {
        if name.is_empty() {
            std::env::remove_var(var);
        } else {
            std::env::set_var(var, name);
        }
    }
}

/// Forces PipeWire to the `rate` sample rate (0: gives it back its usual rate).
///
/// PipeWire runs all sound at a single rate (usually 48 kHz) and resamples the rest; bit-perfect
/// output asks it to follow the track's rate. The setting (`clock.force-rate`) only lasts for
/// the PipeWire session.
pub fn force_rate(rate: u32) {
    let _ = std::process::Command::new("pw-metadata")
        .args(["-n", "settings", "0", "clock.force-rate", &rate.to_string()])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status();
}

/// True if `pw-metadata` (PipeWire) is available: without it, bit-perfect output cannot prevent
/// resampling.
pub fn can_force_rate() -> bool {
    static OK: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *OK.get_or_init(|| {
        std::process::Command::new("pw-metadata")
            .arg("--version")
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .is_ok_and(|s| s.success())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_pactl() {
        let json = br#"[{"index":64,"name":"alsa_output.hdmi","description":"HDMI","state":"SUSPENDED"},
            {"index":66,"name":"alsa_output.usb","description":"  ","state":"RUNNING"}]"#;
        let list = parse(json);
        assert_eq!(list.len(), 2);
        assert_eq!(list[0].description, "HDMI");
        assert_eq!(list[1].description, "alsa_output.usb");
        assert!(parse(b"pas du json").is_empty());
    }
}
