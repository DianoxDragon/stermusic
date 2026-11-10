//! Choix de la sortie audio (haut-parleurs, casque USB, HDMI…).
//!
//! Le son passe par le greffon ALSA de PipeWire (sortie « default » de rodio). Ce greffon lit la
//! variable `PIPEWIRE_NODE` à chaque ouverture de la sortie : il suffit de la poser avant
//! d'ouvrir le flux pour l'envoyer vers la sortie choisie, sans court-circuiter PipeWire (le
//! mixage, le volume de Plasma et les autres applications continuent de fonctionner). Sans
//! PipeWire, `PULSE_SINK` fait la même chose pour PulseAudio.
//!
//! La liste des sorties vient de `pactl` (pipewire-pulse), qui donne leur nom lisible.

use serde::Deserialize;

#[derive(Clone, Debug, PartialEq)]
pub struct Output {
    /// Nom interne (`alsa_output.usb-…`), celui que PipeWire attend.
    pub name: String,
    /// Nom lisible (« Casque Yeti Stereo »).
    pub description: String,
}

#[derive(Deserialize)]
struct Sink {
    name: String,
    #[serde(default)]
    description: String,
}

/// Les sorties audio du système, dans l'ordre de `pactl` ; vide si `pactl` est absent.
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

/// Oriente les prochaines ouvertures de la sortie audio vers `name` (vide : sortie par défaut).
///
/// Appelé seulement quand le choix change (au lancement, ou depuis les paramètres) : la
/// variable d'environnement est lue par le greffon ALSA à l'ouverture du flux.
pub fn route_to(name: &str) {
    for var in ["PIPEWIRE_NODE", "PULSE_SINK"] {
        if name.is_empty() {
            std::env::remove_var(var);
        } else {
            std::env::set_var(var, name);
        }
    }
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
