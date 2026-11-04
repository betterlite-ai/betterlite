use std::ops::Sub as _;

use lopdf::Object;

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
        ] = operands
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
        self.offset_x = horizontal.mul_add(
            self.horizontal_x,
            vertical.mul_add(self.vertical_x, self.offset_x),
        );
        self.offset_y = horizontal.mul_add(
            self.horizontal_y,
            vertical.mul_add(self.vertical_y, self.offset_y),
        );
    }

    pub(super) const fn combine(self, other: Self) -> Self {
        Self {
            horizontal_x: self.horizontal_x.mul_add(
                other.horizontal_x,
                self.horizontal_y.mul_add(other.vertical_x, 0.0_f64),
            ),
            horizontal_y: self.horizontal_x.mul_add(
                other.horizontal_y,
                self.horizontal_y.mul_add(other.vertical_y, 0.0_f64),
            ),
            vertical_x: self.vertical_x.mul_add(
                other.horizontal_x,
                self.vertical_y.mul_add(other.vertical_x, 0.0_f64),
            ),
            vertical_y: self.vertical_x.mul_add(
                other.horizontal_y,
                self.vertical_y.mul_add(other.vertical_y, 0.0_f64),
            ),
            offset_x: self.offset_x.mul_add(
                other.horizontal_x,
                self.offset_y.mul_add(other.vertical_x, other.offset_x),
            ),
            offset_y: self.offset_x.mul_add(
                other.horizontal_y,
                self.offset_y.mul_add(other.vertical_y, other.offset_y),
            ),
        }
    }
}

pub(super) struct Position {
    pub(super) text: Matrix,
    pub(super) graphics: Matrix,
    previous: Matrix,
    pub(super) height: f64,
    span: Option<(bool, f64, f64)>,
}

impl Default for Position {
    fn default() -> Self {
        Self {
            text: IDENTITY,
            graphics: IDENTITY,
            previous: IDENTITY,
            height: 0.0_f64,
            span: None,
        }
    }
}

impl Position {
    pub(super) fn check(&mut self, output: &mut String, font_size: f64) {
        let current = self.text.combine(self.graphics);
        let horizontal_axis = current.vertical_y.abs() <= 0.000_001_f64;
        let axis = current.axis(horizontal_axis);
        let previous_axis = self.previous.axis(horizontal_axis);
        let (low, high) = match self.span {
            Some((direction, low, high)) if direction == horizontal_axis => (low, high),
            _ => (previous_axis, previous_axis),
        };
        let distance = axis.sub(high).max(low.sub(axis)).max(0.0_f64);
        let old_height = self.height.mul_add(
            self.previous.vertical_x.hypot(self.previous.vertical_y),
            0.0_f64,
        );
        let height = font_size.mul_add(current.vertical_x.hypot(current.vertical_y), 0.0_f64);
        if distance > 0.8_f64.mul_add(old_height.min(height), 0.0_f64) {
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
    match object {
        Object::Integer(value) => value.to_string().parse::<f64>().ok(),
        Object::Real(value) => Some(f64::from(*value)),
        Object::Boolean(_)
        | Object::Name(_)
        | Object::String(_, _)
        | Object::Array(_)
        | Object::Dictionary(_)
        | Object::Stream(_)
        | Object::Null
        | Object::Reference(_) => None,
    }
}
