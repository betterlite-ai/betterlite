#![expect(
    clippy::float_arithmetic,
    clippy::suspicious_operation_groupings,
    reason = "pypdf compares float text-space coordinates; affine products combine different axes"
)]

mod tests;

use lopdf::Object;
use lopdf::content::Operation;

#[derive(Clone, Copy)]
pub(super) struct Matrix {
    horizontal_x: f64,
    horizontal_y: f64,
    vertical_x: f64,
    vertical_y: f64,
    offset_x: f64,
    offset_y: f64,
}

pub(super) const IDENTITY: Matrix = Matrix {
    horizontal_x: 1.0_f64,
    horizontal_y: 0.0_f64,
    vertical_x: 0.0_f64,
    vertical_y: 1.0_f64,
    offset_x: 0.0_f64,
    offset_y: 0.0_f64,
};

impl Matrix {
    const fn axis(self, horizontal: bool) -> f64 {
        if horizontal {
            self.offset_x
        } else {
            self.offset_y
        }
    }

    pub(super) fn read(operands: &[Object]) -> Option<Self> {
        let [
            horizontal_x,
            horizontal_y,
            vertical_x,
            vertical_y,
            offset_x,
            offset_y,
        ] = operands.get(..6)?
        else {
            return None;
        };
        Some(Self {
            horizontal_x: number(horizontal_x)?,
            horizontal_y: number(horizontal_y)?,
            vertical_x: number(vertical_x)?,
            vertical_y: number(vertical_y)?,
            offset_x: number(offset_x)?,
            offset_y: number(offset_y)?,
        })
    }

    pub(super) const fn translate(&mut self, horizontal: f64, vertical: f64) {
        self.offset_x += horizontal * self.horizontal_x + vertical * self.vertical_x;
        self.offset_y += horizontal * self.horizontal_y + vertical * self.vertical_y;
    }

    pub(super) const fn combine(self, other: Self) -> Self {
        Self {
            horizontal_x: self.horizontal_x * other.horizontal_x
                + self.horizontal_y * other.vertical_x,
            horizontal_y: self.horizontal_x * other.horizontal_y
                + self.horizontal_y * other.vertical_y,
            vertical_x: self.vertical_x * other.horizontal_x + self.vertical_y * other.vertical_x,
            vertical_y: self.vertical_x * other.horizontal_y + self.vertical_y * other.vertical_y,
            offset_x: self.offset_x * other.horizontal_x
                + self.offset_y * other.vertical_x
                + other.offset_x,
            offset_y: self.offset_x * other.horizontal_y
                + self.offset_y * other.vertical_y
                + other.offset_y,
        }
    }
}

pub(super) struct Position {
    pub(super) text: Matrix,
    pub(super) graphics: Matrix,
    previous: Matrix,
    pub(super) height: f64,
    pub(super) leading: f64,
    span: Option<(bool, f64, f64)>,
}

impl Default for Position {
    fn default() -> Self {
        Self {
            text: IDENTITY,
            graphics: IDENTITY,
            previous: IDENTITY,
            height: 0.0_f64,
            leading: 0.0_f64,
            span: None,
        }
    }
}

impl Position {
    pub(super) fn apply(&mut self, operation: &Operation, output: &mut String, font_size: f64) {
        match operation.operator.as_str() {
            "TL" => {
                self.leading = operation
                    .operands
                    .first()
                    .and_then(number)
                    .unwrap_or(0.0_f64);
            }
            "Td" | "TD" => {
                self.move_by(&operation.operands, operation.operator == "TD");
                self.check(output, font_size);
            }
            "Tm" => {
                self.text = Matrix::read(&operation.operands).unwrap_or(IDENTITY);
                self.check(output, font_size);
            }
            "T*" => self.next_line(output, font_size),
            "BT" => self.text = IDENTITY,
            "cm" => {
                self.graphics = Matrix::read(&operation.operands)
                    .map_or(IDENTITY, |matrix| matrix.combine(self.graphics));
            }
            _ => {}
        }
    }

    fn move_by(&mut self, operands: &[Object], set_leading: bool) {
        let horizontal = operands.first().and_then(number).unwrap_or(0.0_f64);
        let vertical = operands.get(1).and_then(number);
        if set_leading && let Some(value) = vertical {
            self.leading = -value;
        }
        self.text.translate(horizontal, vertical.unwrap_or(0.0_f64));
    }

    pub(super) fn next_line(&mut self, output: &mut String, font_size: f64) {
        self.text.translate(0.0_f64, -self.leading);
        self.check(output, font_size);
    }

    pub(super) fn check(&mut self, output: &mut String, font_size: f64) {
        let current = self.text.combine(self.graphics);
        let horizontal_axis = current.vertical_y.abs() <= 0.000_001_f64;
        let axis = current.axis(horizontal_axis);
        let previous_axis = self.previous.axis(horizontal_axis);
        let (low, high) = match self.span {
            Some((direction, low, high)) if direction == horizontal_axis => (low, high),
            _ => (previous_axis, previous_axis),
        };
        let distance = (axis - high).max(low - axis).max(0.0_f64);
        let old_height = self.height * self.previous.vertical_x.hypot(self.previous.vertical_y);
        let height = font_size * current.vertical_x.hypot(current.vertical_y);
        if distance > 0.8_f64 * old_height.min(height) {
            if !output.is_empty() && !output.ends_with('\n') {
                output.push('\n');
            }
            self.span = Some((horizontal_axis, axis, axis));
        } else if output.is_empty()
            || output.ends_with('\n')
            || self
                .span
                .is_none_or(|(direction, _, _)| direction != horizontal_axis)
        {
            self.span = Some((horizontal_axis, axis, axis));
        } else {
            self.span = Some((horizontal_axis, low.min(axis), high.max(axis)));
        }
        self.previous = current;
    }
}

pub(super) fn number(object: &Object) -> Option<f64> {
    object.as_i64().map_or_else(
        |_| object.as_f32().ok().map(f64::from),
        |integer| i32::try_from(integer).ok().map(f64::from),
    )
}
