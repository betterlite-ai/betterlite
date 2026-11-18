use super::{ConnectionOptions, quote_identifier};

#[derive(Debug)]
pub struct Ownership {
    pub(super) resource: Resource,
    pub(super) admin: Box<ConnectionOptions>,
}

#[derive(Debug)]
pub(super) struct Resource(pub(super) String);

impl Resource {
    pub(super) fn drop_statement(&self) -> String {
        format!(
            "DROP DATABASE IF EXISTS {} WITH (FORCE)",
            quote_identifier(&self.0)
        )
    }
}
