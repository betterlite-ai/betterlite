use thiserror::Error;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum DefinitionsError {
    #[error("unpack_defs: inlined schema exceeded the {limit}-byte budget")]
    ExpansionBudgetExceeded { limit: usize },
}
