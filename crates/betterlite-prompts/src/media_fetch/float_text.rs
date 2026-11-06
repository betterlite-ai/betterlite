pub(crate) fn float_text(value: f64) -> String {
    if value.is_nan() {
        return "nan".to_owned();
    }
    // Python prints a float in fixed notation for magnitudes in [1e-4, 1e16) and in scientific notation outside it
    let magnitude = value.abs();
    if magnitude != 0.0_f64 && !(1e-4_f64..1e16_f64).contains(&magnitude) {
        let scientific = format!("{value:e}");
        if let Some((mantissa, exponent)) = scientific.split_once('e')
            && let Ok(power) = exponent.parse::<i32>()
        {
            let sign = if power < 0_i32 { '-' } else { '+' };
            return format!("{mantissa}e{sign}{:02}", power.unsigned_abs());
        }
        return scientific;
    }
    let fixed = format!("{value}");
    if fixed.contains('.') {
        fixed
    } else {
        format!("{fixed}.0")
    }
}
