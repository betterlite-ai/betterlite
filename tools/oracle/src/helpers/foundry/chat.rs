use betterlite_foundry::chat::fields::required_params;
use serde_json::{Map, Value, json};
use tokio::runtime::Builder;

use betterlite_oracle_helper_api::error::{HelperError, OrHelperError as _};
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};

mod fixture;
mod input;
mod reports;
mod tests;

use fixture::Installed;
use input::Input;

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let runtime = Builder::new_current_thread().build().or_value_error()?;
        let mut installed = Installed::new(input);
        let mut out = Map::new();
        insert_section(&mut out, "urls", input.urls.as_deref(), reports::url_report);
        insert_section(&mut out, "supported", input.supported.as_deref(), |model| {
            reports::supported_report(&runtime, &mut installed, model)
        });
        insert_section(&mut out, "maps", input.maps.as_deref(), |call| {
            reports::map_report(&runtime, &mut installed, call)
        });
        insert_section(
            &mut out,
            "requests",
            input.requests.as_deref(),
            reports::request_report,
        );
        insert_section(
            &mut out,
            "responses",
            input.responses.as_deref(),
            reports::response_report,
        );
        insert_section(&mut out, "providers", input.providers.as_deref(), |call| {
            reports::provider_report(&installed, call)
        });
        insert_auths(input, &mut out)?;
        if input.fields == Some(true) {
            out.insert(String::from("fields"), json!(required_params()));
        }
        Ok(Value::Object(out))
    }
}

fn insert_section<T, Report>(
    answer: &mut Map<String, Value>,
    name: &str,
    calls: Option<&[T]>,
    report: Report,
) where
    Report: FnMut(&T) -> Value,
{
    if let Some(found) = calls {
        answer.insert(String::from(name), found.iter().map(report).collect());
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
