use super::seconds::{DEFAULT_REQUEST_TIMEOUT, Seconds};

// litellm.request_timeout always holds the package default, so the flag is what separates a user's 6000 from the default
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GlobalRequestTimeout {
    pub seconds: Seconds,
    pub explicitly_set: bool,
}

impl GlobalRequestTimeout {
    #[must_use]
    pub fn configured(self) -> Option<Seconds> {
        if self.explicitly_set {
            return Some(self.seconds);
        }
        if self.seconds.0.to_bits() != DEFAULT_REQUEST_TIMEOUT.0.to_bits() {
            return Some(self.seconds);
        }
        None
    }
}
