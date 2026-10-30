#[derive(Clone, Copy, Debug, Default)]
pub struct NoOpMetric;

pub trait Metric: Sized {
    #[must_use]
    fn labels(&self, labels: &[(&str, &str)]) -> Self;
    fn inc(&self, value: u64);
    fn set(&self, value: u64);
    fn observe(&self, value: u64);
}

impl Metric for NoOpMetric {
    fn labels(&self, _labels: &[(&str, &str)]) -> Self {
        *self
    }
    fn inc(&self, _value: u64) {}
    fn set(&self, _value: u64) {}
    fn observe(&self, _value: u64) {}
}
