use betterlite_foundry::chat::fields::required_params;
use serde_json::{Map, Value, json};
use tokio::runtime::Builder;

use betterlite_oracle_helper_api::error::{HelperError, OrHelperError as _};
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};

mod fixture;
mod input;
mod reports;
// the Anthropic messages helper already defines the offline token sources
#[path = "anthropic_messages/sources.rs"]
mod sources;
mod tests;

use fixture::Installed;
use input::Input;

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let runtime = Builder::new_current_thread().build().or_value_error()?;
        let mut installed = Installed::new(input);
        let mut answer = Map::new();
        insert_section(
            &mut answer,
            "urls",
            input.urls.as_deref(),
            reports::url_report,
        );
        insert_section(
            &mut answer,
            "supported",
            input.supported.as_deref(),
            |model| reports::supported_report(&runtime, &mut installed, model),
        );
        insert_section(&mut answer, "maps", input.maps.as_deref(), |call| {
            reports::map_report(&runtime, &mut installed, call)
        });
        insert_section(
            &mut answer,
            "requests",
            input.requests.as_deref(),
            reports::request_report,
        );
        insert_section(
            &mut answer,
            "responses",
            input.responses.as_deref(),
            reports::response_report,
        );
        insert_section(
            &mut answer,
            "providers",
            input.providers.as_deref(),
            |call| reports::provider_report(&installed, call),
        );
        insert_auths(input, &mut answer)?;
        if input.fields == Some(true) {
            answer.insert(String::from("fields"), json!(required_params()));
        }
        Ok(Value::Object(answer))
    }
}

fn insert_section<T, Report: FnMut(&T) -> Value>(
    answer: &mut Map<String, Value>,
    name: &str,
    calls: Option<&[T]>,
    report: Report,
) {
    if let Some(calls) = calls {
        answer.insert(String::from(name), calls.iter().map(report).collect());
    }
}

fn insert_auths(input: &Input, answer: &mut Map<String, Value>) -> Result<(), Box<HelperError>> {
    let Some(calls) = &input.auths else {
        return Ok(());
    };
    let values = calls
        .iter()
        .map(reports::auth_report)
        .collect::<Result<Vec<_>, _>>()?;
    answer.insert(String::from("auths"), Value::Array(values));
    Ok(())
}
