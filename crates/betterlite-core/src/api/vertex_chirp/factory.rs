use crate::api::realtime_contracts::transport::RealtimeBackend;

use super::target::SpeechStreamingTarget;

pub trait ChirpBackendFactory: Send + Sync {
    type Backend: RealtimeBackend;

    fn open(&self, target: SpeechStreamingTarget) -> Self::Backend;
}
