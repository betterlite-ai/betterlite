use betterlite_openai::batches::records::BatchObject;

use crate::api::batch_provider_create::BatchProviderCreateCall;
use crate::api::batch_provider_create::error::BatchProviderCreateError;

// The four provider entry points create_batch calls, behind one typed request per branch
pub trait BatchCreateTransport {
    fn create(
        &self,
        call: &BatchProviderCreateCall,
    ) -> Result<BatchObject, BatchProviderCreateError>;
}
