mod tests;

use crate::api::realtime_connect::RealtimeConnectionSettings;
use crate::api::realtime_connect::load_settings as load_connection_settings;
use crate::loader::{ConfigError, EnvironmentLookup};
use crate::sections::Section;
use crate::sections::openai_realtime::Raw as RealtimeRaw;

// the shared connection settings plus the websocket message cap the probe hands websockets.connect
#[derive(Debug, Clone, PartialEq)]
pub struct RealtimeProbeSettings {
    pub connection: RealtimeConnectionSettings,
    pub max_message_size_bytes: usize,
}

// The whole config loads once, so an unsupported key fails here as it does for every other section.
pub fn load_settings<L>(source: &str, lookup: L) -> Result<RealtimeProbeSettings, ConfigError>
where
    L: EnvironmentLookup + Sync,
{
    let realtime = RealtimeRaw::new().convert(&lookup)?;
    let connection = load_connection_settings(source, lookup)?;
    Ok(RealtimeProbeSettings {
        connection,
        max_message_size_bytes: realtime.max_message_size_bytes,
    })
}
