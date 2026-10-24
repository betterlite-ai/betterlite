use schemars::JsonSchema;
use serde::Deserialize;

use crate::spec::report::ReportDeviation;

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[schemars(
    title = "betterlite deviation log",
    description = "Where one task's code deliberately differs from the pinned LiteLLM, kept in docs/deviations/<task id>.yaml. The worker writes the same entries as its report's deviations."
)]
pub(crate) struct DeviationLog {
    #[schemars(regex(pattern = "^B([0-9]{3}|[1-9][0-9]{3,5})$"))]
    pub(crate) task: String,
    pub(crate) deviations: Vec<ReportDeviation>,
}
