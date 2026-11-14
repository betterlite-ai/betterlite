use thiserror::Error;

#[derive(Debug, Error)]
pub enum RecordsError {
    #[error("the {field} option must be a boolean")]
    Boolean { field: &'static str },
    #[error("the {field} option must be a non-negative integer")]
    Integer { field: &'static str },
    #[error("the {field} option must be a string")]
    Text { field: &'static str },
    #[error("the {field} option must be a list of strings")]
    Granularities { field: &'static str },
    #[error("the {field} option must be a number")]
    Number { field: &'static str },
    #[error("the {field} value cannot be written as JSON")]
    Value {
        field: &'static str,
        #[source]
        source: serde_json::Error,
    },
    #[error("the {field} timings cannot be divided into seconds")]
    Timing { field: &'static str },
}
