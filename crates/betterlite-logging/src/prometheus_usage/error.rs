use std::error::Error as StdError;

use rust_decimal::Decimal;

use crate::prometheus::bounded_series::CounterEmissionError;
use crate::prometheus_label_projection::prometheus_label_factory::LabelFactoryError;

#[derive(Debug, thiserror::Error)]
pub enum PrometheusUsageError<MetricError>
where
    MetricError: StdError + 'static,
{
    #[error(transparent)]
    Counter(#[from] CounterEmissionError<MetricError>),
    #[error(transparent)]
    Labels(#[from] LabelFactoryError),
    #[error("a prometheus counter cannot take the negative amount {amount}")]
    NegativeAmount { amount: Decimal },
}
