#[derive(Clone, Copy, Debug, Default)]
pub struct NoOpMetric;

impl NoOpMetric {
    #[must_use]
    pub const fn new() -> Self {
        Self
    }

    #[must_use]
    pub const fn labels(&self) -> Self {
        Self
    }

    pub const fn inc(&self) {}

    pub const fn set(&self) {}

    pub const fn observe(&self) {}
}
