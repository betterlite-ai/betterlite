pub mod apply;
pub mod batch;
pub mod error;
pub mod increment;
pub mod initialization;
pub mod invalidation;
pub mod layers;
pub mod pipeline;
pub mod settlement;
pub mod source;
pub mod ttl;
pub mod warm;

mod tests;

use std::sync::Arc;

use crate::spend_reseed::cache::ReseedCache;
use crate::spend_reseed::cache::dual::ReseedCacheError;
use crate::spend_reseed::reseed::SpendCounterReseed;
use crate::spend_reseed::rows::SpendCounterRows;

use self::batch::CounterBatch;
use self::layers::CounterLayers;
use self::source::SourceSpendCache;

// LiteLLM's proxy_server spend counter writes over the layers B1736 and B1718 build, a free function per area since the workspace denies multiple_inherent_impl
pub struct SpendCounterWrites<Layers, Rows>
where
    Layers: ReseedCache<Error = ReseedCacheError> + CounterLayers,
    Rows: SpendCounterRows,
{
    pub layers: Arc<Layers>,
    pub batch: Arc<dyn CounterBatch>,
    pub reseed: Arc<SpendCounterReseed<Rows>>,
    pub sources: Arc<dyn SourceSpendCache>,
}
