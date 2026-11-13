use uuid::Uuid;

// str(uuid.uuid4()) for the step id _prepare_log_data generates, injected so a test fixes the value.
pub trait IdSource: Send + Sync {
    fn next_id(&self) -> String;
}

#[derive(Debug, Default)]
pub struct SystemIds;

impl IdSource for SystemIds {
    fn next_id(&self) -> String {
        Uuid::new_v4().to_string()
    }
}
