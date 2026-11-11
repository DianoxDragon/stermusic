//! Compatibilité avec les distributions moins récentes.
//!
//! glibc 2.43 a publié de nouvelles versions de quelques fonctions mathématiques en simple
//! précision : un binaire compilé sur une distribution récente les réclamait, et refusait de se
//! lancer ailleurs (Ubuntu 24.04, Debian 12…). Le programme les fournit donc lui-même, en passant
//! par la double précision, que glibc offre depuis toujours : il tourne alors dès glibc 2.35.

#[no_mangle]
pub extern "C" fn acosf(x: f32) -> f32 {
    f64::from(x).acos() as f32
}

#[no_mangle]
pub extern "C" fn asinf(x: f32) -> f32 {
    f64::from(x).asin() as f32
}

#[no_mangle]
pub extern "C" fn atan2f(y: f32, x: f32) -> f32 {
    f64::from(y).atan2(f64::from(x)) as f32
}

#[no_mangle]
pub extern "C" fn log10f(x: f32) -> f32 {
    f64::from(x).log10() as f32
}

#[cfg(test)]
mod tests {
    #[test]
    fn same_results() {
        for i in -100..=100 {
            let x = i as f32 / 100.0;
            assert!((super::acosf(x) - (x as f64).acos() as f32).abs() < 1e-6);
            assert!((super::asinf(x) - (x as f64).asin() as f32).abs() < 1e-6);
            assert!((super::atan2f(x, 0.3) - (x as f64).atan2(0.3) as f32).abs() < 1e-6);
        }
        assert!((super::log10f(1000.0) - 3.0).abs() < 1e-6);
        assert!(super::acosf(2.0).is_nan());
    }
}
