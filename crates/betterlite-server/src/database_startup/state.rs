// The status vocabulary readiness reports; the probe owns the transitions, this module owns the words
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum DatabaseStatus {
    #[default]
    Unknown,
    Connected,
    Disconnected,
}

impl DatabaseStatus {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Unknown => "unknown",
            Self::Connected => "connected",
            Self::Disconnected => "disconnected",
        }
    }
}

// The database state B4448's readiness probe reads; this module never implements its HTTP route.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DatabaseState {
    pub configured: bool,
}

impl DatabaseState {
    // _resolve_public_readiness_db: without a configured database the probe answers "Not connected"
    #[must_use]
    pub const fn public_status(&self, status: DatabaseStatus) -> &'static str {
        if self.configured {
            status.as_str()
        } else {
            "Not connected"
        }
    }
}
