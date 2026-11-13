use std::time::Duration;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OidcHttpTimeouts {
    pub connect: Duration,
    pub read: Duration,
}

impl OidcHttpTimeouts {
    pub const DEFAULT: Self = Self {
        connect: Duration::from_secs(5),
        read: Duration::from_secs(600),
    };
}
