use thiserror::Error;

#[derive(Debug, Error)]
pub enum SummaryError {
    #[error("the estimate of {value} hours does not fit a decimal")]
    Hours {
        value: f64,
        #[source]
        source: rust_decimal::Error,
    },
    #[error("the {figure} does not fit a decimal")]
    Overflow { figure: &'static str },
}
