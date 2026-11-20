use std::sync::Arc;

use betterlite_anthropic::api::messages_errors::decode::WireDecodeLimits;
use betterlite_redact::secret::Redactor;
use futures_util::StreamExt as _;
use futures_util::stream::BoxStream;

use super::call::{SharedCall, StreamAbort};
use super::drive::{Drive, Ports, Step};
use super::framing::ChunkFraming;
use super::passthrough::{NoCleanup, NoRelease, PassThroughHooks};
use super::ports::{CostInjection, ReservationRelease};
use crate::keepalive_interval::PingSetting;
use crate::sse_data_generator::types::{StreamCleanup, StreamHooks};

pub const DEFAULT_LIMITS: WireDecodeLimits = WireDecodeLimits {
    max_json_bytes: 0x0100_0000,
    max_event_bytes: 0x0010_0000,
    max_items: 0x0001_0000,
};

pub struct SharedStreamGenerator<Key, H> {
    pub hooks: Arc<H>,
    pub reservation: Arc<dyn ReservationRelease<Key>>,
    pub cleanup: Arc<dyn StreamCleanup<Key>>,
    pub cost: Option<Arc<dyn CostInjection>>,
    pub redactor: Option<Arc<Redactor>>,
    pub limits: WireDecodeLimits,
    pub anthropic_ping: Option<PingSetting>,
}

impl<Key> SharedStreamGenerator<Key, PassThroughHooks> {
    #[must_use]
    pub fn standalone(redactor: Option<Arc<Redactor>>) -> Self {
        Self {
            hooks: Arc::new(PassThroughHooks),
            reservation: Arc::new(NoRelease),
            cleanup: Arc::new(NoCleanup),
            cost: None,
            redactor,
            limits: DEFAULT_LIMITS,
            anthropic_ping: Some(PingSetting::Seconds(15.0)),
        }
    }
}

impl<Key, H> SharedStreamGenerator<Key, H>
where
    Key: Send + Sync + 'static,
    H: StreamHooks<Key> + 'static,
{
    #[must_use]
    pub fn generate<F>(
        &self,
        call: SharedCall<Key>,
        framing: F,
    ) -> BoxStream<'static, Result<F::Out, StreamAbort>>
    where
        F: ChunkFraming,
    {
        let ports = Ports {
            hooks: Arc::clone(&self.hooks),
            reservation: Arc::clone(&self.reservation),
            cleanup: Arc::clone(&self.cleanup),
            cost: self.cost.as_ref().map(Arc::clone),
            redactor: self.redactor.as_ref().map(Arc::clone),
        };
        async_stream::stream! {
            let mut drive = Drive::start(ports, call, framing);
            loop {
                match drive.pump().await {
                    Step::Write(out) => yield Ok(out),
                    Step::End(tail) => {
                        if let Some(tail) = tail {
                            yield Ok(tail);
                        }
                        drive.finish(true);
                        return;
                    }
                    Step::Failed(ending) => {
                        yield ending;
                        drive.finish(false);
                        return;
                    }
                }
            }
        }
        .boxed()
    }
}
