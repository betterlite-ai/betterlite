use serde_json::{Map, Value};

use betterlite_oracle_helper_api::helper::{Helper, HelperResult};

mod input;
mod reports;
mod tests;

use input::Input;
use reports::{
    catalog_report, property_report, reader_report, response_report, strip_report, version_report,
};

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let mut out = Map::new();
        let sections = [
            section("strips", input.strips.as_deref(), strip_report),
            section("responses", input.responses.as_deref(), response_report),
            section("readers", input.readers.as_deref(), reader_report),
            section("catalog", input.catalog.as_deref(), catalog_report),
            section("versions", input.versions.as_deref(), version_report),
            section("properties", input.properties.as_deref(), property_report),
            section("token_models", input.token_models.as_deref(), |model| {
                reports::token_report(model.as_deref())
            }),
        ];
        for (name, value) in sections.into_iter().flatten() {
            out.insert(String::from(name), value);
        }
        if input.validate == Some(true) {
            out.insert(String::from("validate"), reports::validate_report());
        }
        Ok(Value::Object(out))
    }
}

fn section<'name, Call, Report>(
    name: &'name str,
    calls: Option<&[Call]>,
    report: Report,
) -> Option<(&'name str, Value)>
where
    Report: FnMut(&Call) -> Value,
{
    if let Some(found) = calls {
        return Some((name, found.iter().map(report).collect()));
    }
    None
}
