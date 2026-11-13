#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct BreakerAdmission(u64);

// The gate packs the breaker generation and the call number into the token; callers treat it as opaque.
impl From<u64> for BreakerAdmission {
    fn from(bits: u64) -> Self {
        Self(bits)
    }
}

impl AsRef<u64> for BreakerAdmission {
    fn as_ref(&self) -> &u64 {
        &self.0
    }
}

#[derive(Debug, thiserror::Error)]
#[error("Redis circuit breaker is open, skipping {name}")]
pub struct BreakerOpen {
    pub name: &'static str,
}
