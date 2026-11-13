use betterlite_openai::batches::records::{BatchObject, BatchStatus};

// the batch post-call hook only needs the id and status back out of a provider batch response
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BatchResponseView {
    pub id: String,
    pub status: BatchStatus,
}

impl From<&BatchObject> for BatchResponseView {
    fn from(response: &BatchObject) -> Self {
        Self {
            id: response.id.clone(),
            status: response.status,
        }
    }
}
