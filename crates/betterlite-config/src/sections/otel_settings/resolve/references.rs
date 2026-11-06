use super::super::{ListSetting, RawOtel};
use crate::loader::{ConfigError, EnvironmentLookup};
use crate::resolve::resolve_string;

pub(super) fn resolve<L>(raw: &mut RawOtel, lookup: &L) -> Result<(), ConfigError>
where
    L: EnvironmentLookup,
{
    for (name, field) in [
        ("exporter", &mut raw.exporter),
        ("endpoint", &mut raw.endpoint),
        ("traces_endpoint", &mut raw.traces_endpoint),
        ("headers", &mut raw.headers),
        ("service_name", &mut raw.service_name),
        ("deployment_environment", &mut raw.deployment_environment),
        ("capture_message_content", &mut raw.capture_message_content),
        ("langfuse_span_scope", &mut raw.langfuse_span_scope),
    ] {
        if let Some(value) = field.take() {
            *field = Some(resolve_string(
                value,
                &format!("callback_settings.otel.{name}"),
                lookup,
            )?);
        }
    }
    for (name, field) in [
        ("mapper_names", &mut raw.mapper_names),
        ("baggage_promoted_keys", &mut raw.baggage_promoted_keys),
        ("baggage_metadata_keys", &mut raw.baggage_metadata_keys),
        (
            "baggage_team_metadata_keys",
            &mut raw.baggage_team_metadata_keys,
        ),
    ] {
        let path = format!("callback_settings.otel.{name}");
        match field {
            Some(ListSetting::Csv(text)) => *text = resolve_string(text.clone(), &path, lookup)?,
            Some(ListSetting::List(items)) => {
                for item in items {
                    *item = resolve_string(item.clone(), &path, lookup)?;
                }
            }
            None => {}
        }
    }
    Ok(())
}
