use thiserror::Error;

// The selector reads a limit or a price as a number; LiteLLM instead adds the value to a number and raises a TypeError.
#[derive(Debug, Error, Eq, PartialEq)]
pub enum CostRoutingError {
    #[error("deployment {deployment} has a {field} that is not a number")]
    NonNumeric {
        deployment: String,
        field: &'static str,
    },
}

impl CostRoutingError {
    // the field the routing error names, carried by the pipeline's existing metric error
    #[must_use]
    pub const fn field(&self) -> &'static str {
        match self {
            Self::NonNumeric { field, .. } => field,
        }
    }
}
