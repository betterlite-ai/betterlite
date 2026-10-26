use betterlite_context::request_records::record::RequestRecord;
use betterlite_context::request_records::{GatewayCacheAttribution, RequestRecordProjection};
use serde_json::json;
use std::collections::BTreeSet;

mod tests;

#[derive(Debug, Clone, serde::Deserialize)]
#[serde(tag = "function", rename_all = "snake_case", deny_unknown_fields)]
#[rustfmt::skip]
pub(crate) enum Input { SafeCopy { record: RequestRecord, safe_memory_mode: bool }, IndependentSnapshot { record: RequestRecord }, FilterParameters { record: RequestRecord, max_depth: i32 }, FilterInternal { record: RequestRecord, additional: Vec<String> }, GatewayInjection { record: RequestRecord, steps: Vec<GatewayStep> } }

#[derive(Debug, Clone, serde::Deserialize)]
pub(crate) struct GatewayStep {
    added: i32,
    every_deployment: bool,
    model_info: RequestRecord,
}

pub(crate) struct Op;

fn run_gateway(record: &RequestRecord, steps: &[GatewayStep]) -> RequestRecord {
    let mut changed = record.clone();
    for step in steps {
        if let RequestRecord::Object(fields) = &mut changed {
            fields.insert("model_info".to_owned(), step.model_info.clone());
        }
        GatewayCacheAttribution::record(&mut changed, step.added, step.every_deployment);
    }
    changed
}

impl crate::helpers::Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> crate::helpers::HelperResult {
        let result = match input {
            Input::SafeCopy {
                record,
                safe_memory_mode,
            } => {
                let mut changed = record.clone();
                RequestRecordProjection::safe_copy(&mut changed, *safe_memory_mode)
                    .map_err(|error| {
                        let class = match &error {
                            betterlite_context::request_records::RequestRecordError::MetadataMembership { .. } => "TypeError",
                            betterlite_context::request_records::RequestRecordError::MetadataMutation { .. } => "AttributeError",
                        };
                        Box::new(crate::helpers::HelperError {
                            class: class.to_owned(),
                            status: None,
                            error_type: None,
                            param: None,
                            code: None,
                            message: error.to_string(),
                        })
                    })?
                    .into_owned()
            }
            Input::IndependentSnapshot { record } => {
                RequestRecordProjection::independent_snapshot(record)
            }
            Input::FilterParameters { record, max_depth } => {
                RequestRecordProjection::filter_parameters(record.clone(), *max_depth)
            }
            Input::FilterInternal { record, additional } => {
                RequestRecordProjection::filter_internal(
                    record.clone(),
                    &additional.iter().cloned().collect::<BTreeSet<_>>(),
                )
            }
            Input::GatewayInjection { record, steps } => run_gateway(record, steps),
        };
        Ok(json!(result))
    }
}
