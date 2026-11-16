use betterlite_config::sections::database_startup::startup::StartupSettings;

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
    configured: bool,
}

impl DatabaseState {
    // proxy_server builds prisma_client only from DATABASE_URL, so DIRECT_URL alone is no database
    #[must_use]
    pub const fn from_settings(settings: &StartupSettings) -> Self {
        Self {
            configured: settings.database_url.is_some(),
        }
    }

    #[must_use]
    pub const fn is_configured(&self) -> bool {
        self.configured
    }

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
