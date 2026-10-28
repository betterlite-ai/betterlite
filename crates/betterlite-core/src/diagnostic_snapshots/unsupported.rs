use serde::Serialize;

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct DiagnosticError {
    #[serde(rename = "type")]
    pub error_type: String,
    pub code: String,
    pub message: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct UnsupportedSnapshot {
    pub error: DiagnosticError,
}

impl UnsupportedSnapshot {
    #[must_use]
    pub fn gc_statistics() -> Self {
        let measurement = "GC statistics";
        Self::new(measurement)
    }

    #[must_use]
    pub fn object_type_counts() -> Self {
        let measurement = "object type counts";
        Self::new(measurement)
    }

    #[must_use]
    pub fn uncollectable_objects() -> Self {
        let measurement = "uncollectable objects";
        Self::new(measurement)
    }

    fn new(measurement: &str) -> Self {
        let error_type = "unsupported_runtime".to_owned();
        let code = "unsupported_runtime".to_owned();
        Self {
            error: DiagnosticError {
                error_type,
                code,
                message: format!("{measurement} requires CPython runtime introspection"),
            },
        }
    }
}
