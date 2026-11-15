use uuid::Uuid;

// LiteLLM names failure and custom objects after uuid.uuid4().hex, so a source hands back 32 lowercase hex digits
pub trait IdSource: Send + Sync {
    fn uuid_hex(&self) -> String;
}

#[derive(Clone, Copy, Debug, Default)]
pub struct RandomIds;

impl IdSource for RandomIds {
    fn uuid_hex(&self) -> String {
        Uuid::new_v4().simple().to_string()
    }
}

mod tests;
