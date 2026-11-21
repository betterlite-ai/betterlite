#![cfg(test)]

use super::format;

#[test]
fn keeps_signed_zero_and_small_integers() {
    assert_eq!(format(0.0_f64), "0.0");
    assert_eq!(format(-0.0_f64), "-0.0");
    assert_eq!(format(1.0_f64), "1.0");
    assert_eq!(format(-2.5_f64), "-2.5");
    assert_eq!(format(100.0_f64), "100.0");
}

#[test]
fn renders_named_nonfinite_values() {
    assert_eq!(format(f64::NAN), "NaN");
    assert_eq!(format(f64::INFINITY), "Infinity");
    assert_eq!(format(f64::NEG_INFINITY), "-Infinity");
}

#[test]
fn uses_fixed_notation_inside_pythons_window() {
    assert_eq!(format(0.5_f64), "0.5");
    assert_eq!(format(0.0001_f64), "0.0001");
    assert_eq!(format(1.2345_f64), "1.2345");
    assert_eq!(format(0.1_f64), "0.1");
    assert_eq!(format(1e15_f64), "1000000000000000.0");
    assert_eq!(format(1234567890123456.0_f64), "1234567890123456.0");
}

#[test]
fn switches_to_scientific_outside_pythons_window() {
    assert_eq!(format(0.00001_f64), "1e-05");
    assert_eq!(format(0.000025_f64), "2.5e-05");
    assert_eq!(format(0.0000001_f64), "1e-07");
    assert_eq!(format(1e16_f64), "1e+16");
    assert_eq!(format(1e17_f64), "1e+17");
    assert_eq!(format(1e20_f64), "1e+20");
}

#[test]
fn keeps_the_shortest_round_tripping_digits() {
    assert_eq!(
        format(1.7976931348623157e308_f64),
        "1.7976931348623157e+308"
    );
    assert_eq!(format(5e-324_f64), "5e-324");
    assert_eq!(format(1.2345678901234568e16_f64), "1.2345678901234568e+16");
}

#[test]
fn pads_a_single_exponent_digit() {
    assert_eq!(format(1e-7_f64), "1e-07");
    assert_eq!(format(1.5e-7_f64), "1.5e-07");
    assert_eq!(format(1e-5_f64), "1e-05");
}
