use uuid::Uuid;

pub trait IdSource: Send + Sync {
    fn next_id(&self) -> Uuid;
}

#[derive(Debug, Default)]
pub struct SystemIds;

impl IdSource for SystemIds {
    fn next_id(&self) -> Uuid {
        Uuid::new_v4()
    }
}
