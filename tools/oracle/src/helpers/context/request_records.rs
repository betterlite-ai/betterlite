use betterlite_context::request_records::{
    GatewayCacheAttribution, RequestRecord, RequestRecordProjection,
};
use serde_json::json;
use std::collections::BTreeSet;

mod tests;

#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
#[serde(tag = "function", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Input {
    SafeCopy {
        record: RequestRecord,
        safe_memory_mode: bool,
    },
    IndependentSnapshot {
        record: RequestRecord,
    },
    FilterParameters {
        record: RequestRecord,
        max_depth: Option<i32>,
    },
    FilterInternal {
        record: RequestRecord,
        additional: Vec<String>,
    },
    GatewayInjection {
        record: RequestRecord,
        added: i32,
        every_deployment: bool,
    },
}

pub(crate) struct Op;

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        let result = match input {
            Input::SafeCopy {
                record,
                safe_memory_mode,
            } => RequestRecordProjection::safe_copy(record.clone(), *safe_memory_mode),
            Input::IndependentSnapshot { record } => {
                RequestRecordProjection::independent_snapshot(record)
            }
            Input::FilterParameters { record, max_depth } => {
                RequestRecordProjection::filter_parameters(
                    record.clone(),
                    max_depth.unwrap_or(20_i32),
                )
            }
            Input::FilterInternal { record, additional } => {
                RequestRecordProjection::filter_internal(
                    record.clone(),
                    &additional.iter().cloned().collect::<BTreeSet<_>>(),
                )
            }
            Input::GatewayInjection {
                record,
                added,
                every_deployment,
            } => {
                let mut changed = record.clone();
                GatewayCacheAttribution::record(&mut changed, *added, *every_deployment);
                changed
            }
        };
        Ok(json!(result))
    }
}
