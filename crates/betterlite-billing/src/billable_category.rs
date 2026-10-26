mod tests;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BillableCategory {
    Llm,
    Mcp,
    A2a,
}

impl BillableCategory {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Llm => "llm",
            Self::Mcp => "mcp",
            Self::A2a => "a2a",
        }
    }
}
