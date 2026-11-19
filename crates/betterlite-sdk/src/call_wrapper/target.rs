use async_trait::async_trait;

use super::error::CallError;
use super::state::LoggingHandle;
use crate::call_setup::RequestMap;
use crate::call_setup::correlation::CallOutcome;

// The decorated SDK function: its __name__ is the call type, and the retry entry stands for completion_with_retries
#[async_trait]
pub trait CallTarget: Send + Sync {
    fn name(&self) -> &str;

    async fn call(
        &self,
        request: RequestMap,
        logging: Option<&LoggingHandle>,
    ) -> Result<CallOutcome, CallError>;

    async fn retry(
        &self,
        request: RequestMap,
        logging: Option<&LoggingHandle>,
    ) -> Result<CallOutcome, CallError>;
}
