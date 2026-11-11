//! Petites animations interpolées, avec les courbes de SterCode (ease-out, ease-out-expo).
//!
//! Chaque valeur animée est identifiée par un `Id` et mémorisée dans egui : quand la cible
//! change, l'animation repart de la valeur courante vers la nouvelle cible.

use eframe::egui::{Context, Id};

/// Durées de référence de SterCode.
pub const FAST: f32 = 0.12;
pub const NORMAL: f32 = 0.22;
pub const SLOW: f32 = 0.32;

#[derive(Clone, Copy)]
pub enum Easing {
    Out,
    OutExpo,
    /// Dépasse un peu le but avant de s'y poser (un ressort).
    OutBack,
}

impl Easing {
    pub fn apply(self, t: f32) -> f32 {
        let t = t.clamp(0.0, 1.0);
        match self {
            Easing::Out => 1.0 - (1.0 - t).powi(3),
            Easing::OutExpo => {
                if t >= 1.0 {
                    1.0
                } else {
                    1.0 - 2f32.powf(-10.0 * t)
                }
            }
            Easing::OutBack => {
                const C1: f32 = 1.2;
                let u = t - 1.0;
                1.0 + (C1 + 1.0) * u * u * u + C1 * u * u
            }
        }
    }
}

#[derive(Clone, Copy)]
struct Tween {
    from: f32,
    to: f32,
    start: f64,
}

/// Valeur qui glisse vers `target` en `duration` secondes. La première fois, elle part de `initial`.
pub fn value_from(ctx: &Context, id: Id, initial: f32, target: f32, duration: f32, easing: Easing) -> f32 {
    let now = ctx.input(|i| i.time);
    let mut tween = ctx.data_mut(|d| {
        *d.get_temp_mut_or_insert_with(id, || Tween {
            from: initial,
            to: target,
            start: now,
        })
    });
    let current = sample(&tween, now, duration, easing);
    if tween.to != target {
        tween = Tween {
            from: current,
            to: target,
            start: now,
        };
        ctx.data_mut(|d| d.insert_temp(id, tween));
    }
    let value = sample(&tween, now, duration, easing);
    if value != tween.to {
        ctx.request_repaint();
    }
    value
}

/// Comme [`value_from`], sans animation la première fois.
pub fn value(ctx: &Context, id: Id, target: f32, duration: f32, easing: Easing) -> f32 {
    value_from(ctx, id, target, target, duration, easing)
}

/// 0 → 1 quand `on` devient vrai, 1 → 0 quand il redevient faux.
pub fn toggle(ctx: &Context, id: Id, on: bool, duration: f32) -> f32 {
    value(ctx, id, if on { 1.0 } else { 0.0 }, duration, Easing::Out)
}

/// Apparition : 0 → 1 dès la première frame où l'élément est affiché.
pub fn appear(ctx: &Context, id: Id, duration: f32) -> f32 {
    value_from(ctx, id, 0.0, 1.0, duration, Easing::Out)
}

/// Vrai si l'élément `id` n'était pas affiché à l'image précédente (fenêtre qui s'ouvre, état vide
/// qui revient…). À appeler une seule fois par image et par élément.
pub fn fresh(ctx: &Context, id: Id) -> bool {
    let slot = id.with("seen");
    let pass = ctx.cumulative_pass_nr();
    let prev = ctx.data_mut(|d| {
        let prev = d.get_temp::<u64>(slot);
        d.insert_temp(slot, pass);
        prev
    });
    !prev.is_some_and(|p| p + 1 >= pass)
}

/// Comme [`appear`], mais rejouée chaque fois que l'élément réapparaît après avoir disparu.
pub fn appear_each(ctx: &Context, id: Id, duration: f32, easing: Easing) -> f32 {
    if fresh(ctx, id) {
        reset(ctx, id);
    }
    value_from(ctx, id, 0.0, 1.0, duration, easing)
}

/// Vrai quand `key` diffère de la valeur vue à l'appel précédent (jamais au premier appel).
pub fn key_changed(ctx: &Context, id: Id, key: u64) -> bool {
    let slot = id.with("key");
    let before = ctx.data_mut(|d| {
        let before = d.get_temp::<u64>(slot);
        d.insert_temp(slot, key);
        before
    });
    before.is_some_and(|b| b != key)
}

/// Impulsion : 1 au moment où `key` change, qui retombe à 0 en `duration` secondes (pastille qui
/// rebondit quand son nombre change…).
pub fn pulse(ctx: &Context, id: Id, key: u64, duration: f32) -> f32 {
    if key_changed(ctx, id, key) {
        // L'heure est lue avant : `ctx.input` dans `ctx.data_mut` reprendrait le verrou d'egui
        // déjà tenu, et l'interface se figerait (la musique, elle, continue).
        let start = ctx.input(|i| i.time);
        ctx.data_mut(|d| d.insert_temp(id, Tween { from: 1.0, to: 0.0, start }));
    }
    value(ctx, id, 0.0, duration, Easing::Out)
}

/// Oublie une animation (ex. pour rejouer une apparition).
pub fn reset(ctx: &Context, id: Id) {
    ctx.data_mut(|d| d.remove::<Tween>(id));
}

fn sample(tween: &Tween, now: f64, duration: f32, easing: Easing) -> f32 {
    let t = if duration <= 0.0 {
        1.0
    } else {
        ((now - tween.start) as f32 / duration).clamp(0.0, 1.0)
    };
    if t >= 1.0 {
        tween.to
    } else {
        tween.from + (tween.to - tween.from) * easing.apply(t)
    }
}
