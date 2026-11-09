use std::num::{NonZeroU64, NonZeroUsize};

use secrecy::SecretString;
use serde::Deserialize;
use serde_with::{As, DefaultOnNull};

#[derive(Clone, Debug, Deserialize)]
#[serde(default)]
pub struct ZerobusInitParams {
    pub workspace_url: Option<String>,
    pub server_endpoint: Option<String>,
    pub client_id: Option<String>,
    pub client_secret: Option<SecretString>,
    pub table_name: Option<String>,
    pub batch_size: NonZeroUsize,
    pub flush_interval: NonZeroU64,
    #[serde(default, deserialize_with = "As::<DefaultOnNull>::deserialize")]
    pub turn_off_message_logging: bool,
}

impl Default for ZerobusInitParams {
    fn default() -> Self {
        Self {
            workspace_url: None,
            server_endpoint: None,
            client_id: None,
            client_secret: None,
            table_name: None,
            // NonZeroUsize::new returns an Option and the default has no failure path, so the value counts up from MIN
            batch_size: NonZeroUsize::MIN.saturating_add(99),
            flush_interval: NonZeroU64::MIN.saturating_add(9),
            turn_off_message_logging: false,
        }
    }
}
