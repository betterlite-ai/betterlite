use uuid::Uuid;

pub trait ContentIds {
    fn next_id(&mut self) -> String;
}

#[derive(Clone, Copy, Debug, Default)]
pub struct UuidIds;

impl ContentIds for UuidIds {
    fn next_id(&mut self) -> String {
        Uuid::new_v4().to_string()
    }
}
