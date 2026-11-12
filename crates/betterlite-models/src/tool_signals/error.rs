use thiserror::Error;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QuantityField {
    StorageGb,
    Days,
}

impl QuantityField {
    const fn name(self) -> &'static str {
        match self {
            Self::StorageGb => "storage_gb",
            Self::Days => "days",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum ToolSignalError {
    #[error("the {field} file-search quantity is not a finite number", field = field.name())]
    NonFiniteQuantity { field: QuantityField },
    #[error("the {field} file-search quantity is outside the decimal range", field = field.name())]
    QuantityOutOfRange { field: QuantityField },
    #[error("a counter is outside the supported signed 64-bit range")]
    CounterOutOfRange,
    #[error("web_search_options must be a mapping")]
    OptionsNotMapping,
    #[error("the tools value is not iterable")]
    ToolsNotIterable,
}
