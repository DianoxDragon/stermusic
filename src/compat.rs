//! Compatibility with older distributions.
//!
//! glibc 2.43 published new versions of a few single-precision math functions: a binary built
//! on a recent distribution required them, and refused to start anywhere else (Ubuntu 24.04,
//! Debian 12…). The program therefore provides them itself, going through double precision,
//! which glibc has always offered: it then runs on glibc 2.35 and later.

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
