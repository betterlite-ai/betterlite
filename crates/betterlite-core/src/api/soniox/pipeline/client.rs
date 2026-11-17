#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClientKind {
    Sync,
    Async,
}

// LiteLLM keeps a supplied client only when its kind matches the entry point, and builds the other kind otherwise
pub struct SuppliedClient<'client, T> {
    pub kind: ClientKind,
    pub transport: &'client T,
}
