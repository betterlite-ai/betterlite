use uuid::Uuid;

pub trait UuidSource: Send + Sync {
    fn next_id(&self) -> Uuid;
}

#[derive(Debug, Default)]
pub struct SystemUuids;

impl UuidSource for SystemUuids {
    fn next_id(&self) -> Uuid {
        Uuid::new_v4()
    }
}
