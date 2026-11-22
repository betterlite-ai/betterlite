use betterlite_core::api::file_batch_guardrails::records::{
    BatchScanFailure, UnparseableRecord, UnroutableRecord, UnscannableRecord,
};
use betterlite_core::api::file_batch_rewrite::worst::{ScanProblem, worst_problem};
use serde::Deserialize;
use serde_json::{Value, json};

use betterlite_oracle_helper_api::helper::{Helper, HelperResult};

use super::Op;

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Kind {
    Unparseable,
    Unscannable,
    Unroutable,
    Raised,
}

// One flat shape for a refusal and for an exception, the two things LiteLLM's _worst orders; each kind reads its own fields
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Problem {
    line_number: usize,
    kind: Kind,
    custom_id: Option<String>,
    url: Option<String>,
    guardrail_name: Option<String>,
    status: u16,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Scenario {
    problems: Vec<Problem>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    scenarios: Vec<Scenario>,
}

// What the helper keeps of an exception: only its status, since _worst never reads anything else
struct Raised {
    status: u16,
}

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let selected: Vec<Value> = input.scenarios.iter().map(chosen).collect();
        Ok(json!({ "selected": selected }))
    }
}

fn chosen(scenario: &Scenario) -> Value {
    let mut problems: Vec<(usize, ScanProblem<Raised>)> =
        scenario.problems.iter().map(problem).collect();
    let position = worst_problem(&problems);
    position.map_or(Value::Null, |index| {
        let (line_number, problem) = problems.swap_remove(index);
        describe(line_number, &problem)
    })
}

fn problem(item: &Problem) -> (usize, ScanProblem<Raised>) {
    let failure = match item.kind {
        Kind::Unparseable => BatchScanFailure::Unparseable(UnparseableRecord {
            line_number: item.line_number,
        }),
        Kind::Unscannable => BatchScanFailure::Unscannable(UnscannableRecord {
            line_number: item.line_number,
            custom_id: item.custom_id.clone(),
            url: item.url.clone(),
        }),
        Kind::Unroutable => BatchScanFailure::Unroutable(UnroutableRecord {
            line_number: item.line_number,
            custom_id: item.custom_id.clone(),
            guardrail: item.guardrail_name.clone(),
        }),
        Kind::Raised => {
            let raised = Raised {
                status: item.status,
            };
            return (item.line_number, ScanProblem::Raised(raised));
        }
    };
    (item.line_number, ScanProblem::Failure(failure))
}

fn describe(line_number: usize, problem: &ScanProblem<Raised>) -> Value {
    match problem {
        ScanProblem::Raised(raised) => {
            json!({ "line_number": line_number, "kind": "raised", "status": raised.status })
        }
        ScanProblem::Failure(BatchScanFailure::Unparseable(_)) => {
            json!({ "line_number": line_number, "kind": "unparseable" })
        }
        ScanProblem::Failure(BatchScanFailure::Unscannable(record)) => json!({
            "line_number": line_number,
            "kind": "unscannable",
            "custom_id": record.custom_id,
            "url": record.url
        }),
        ScanProblem::Failure(BatchScanFailure::Unroutable(record)) => json!({
            "line_number": line_number,
            "kind": "unroutable",
            "custom_id": record.custom_id,
            "guardrail": record.guardrail
        }),
    }
}
