pub trait IdSource: Send + Sync {
    fn next_id(&self) -> String;
}

// the chunk and tool-call id source is injected so a fixture can pin every id
#[derive(Debug, Default)]
pub struct RandomIds;

impl IdSource for RandomIds {
    fn next_id(&self) -> String {
        uuid::Uuid::new_v4().to_string()
    }
}
