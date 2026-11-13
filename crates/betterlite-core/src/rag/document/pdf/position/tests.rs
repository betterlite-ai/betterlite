#![cfg(test)]

use super::Matrix;

#[test]
fn pdf_affine_translation_preserves_the_basis() {
    let mut matrix = Matrix {
        horizontal_x: 2.0,
        horizontal_y: 3.0,
        vertical_x: 5.0,
        vertical_y: 7.0,
        offset_x: 11.0,
        offset_y: 13.0,
    };
    matrix.translate(17.0, 19.0);
    let actual = [
        matrix.horizontal_x,
        matrix.horizontal_y,
        matrix.vertical_x,
        matrix.vertical_y,
        matrix.offset_x,
        matrix.offset_y,
    ];
    let expected: [f64; 6] = [2.0, 3.0, 5.0, 7.0, 140.0, 197.0];
    assert_eq!(actual.map(f64::to_bits), expected.map(f64::to_bits));
}

#[test]
fn pdf_affine_composition_transforms_both_axes_and_the_origin() {
    let text = Matrix {
        horizontal_x: 2.0,
        horizontal_y: 3.0,
        vertical_x: 5.0,
        vertical_y: 7.0,
        offset_x: 11.0,
        offset_y: 13.0,
    };
    let graphics = Matrix {
        horizontal_x: 17.0,
        horizontal_y: 19.0,
        vertical_x: 23.0,
        vertical_y: 29.0,
        offset_x: 31.0,
        offset_y: 37.0,
    };
    let combined = text.combine(graphics);
    let actual = [
        combined.horizontal_x,
        combined.horizontal_y,
        combined.vertical_x,
        combined.vertical_y,
        combined.offset_x,
        combined.offset_y,
    ];
    let expected: [f64; 6] = [103.0, 125.0, 246.0, 298.0, 517.0, 623.0];
    assert_eq!(actual.map(f64::to_bits), expected.map(f64::to_bits));
}
