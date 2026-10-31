use crate::s3_transport::option_value::OptionValue;
use crate::s3_transport::options::{Toggle, resolve_toggle};
use crate::s3_v2::{Feature, source::Source};
use std::collections::BTreeSet;
use std::num::FpCategory;

pub(super) fn resolve<Config>(
    source: &Source<Config>,
    defaults: &Source<Config>,
) -> BTreeSet<Feature>
where
    Config: Default,
{
    let mut output = BTreeSet::new();
    legacy_flags(source, defaults, &mut output);
    let drop = source
        .drop_on_terminal_error
        .as_ref()
        .or(defaults.drop_on_terminal_error.as_ref());
    if resolve_toggle(Toggle::DropOnTerminalError, drop) {
        output.insert(Feature::DropOnTerminalError);
    }
    if nonzero(defaults.adaptive_concurrency.as_ref())
        || resolve_toggle(
            Toggle::AdaptiveConcurrency,
            source.adaptive_concurrency.as_ref(),
        )
    {
        output.insert(Feature::AdaptiveConcurrency);
    }
    if nonzero(defaults.batch_file_upload.as_ref())
        || resolve_toggle(Toggle::BatchFileUpload, source.batch_file_upload.as_ref())
    {
        output.insert(Feature::BatchFileUpload);
    }
    output
}

// `s3_adaptive_concurrency or resolve_s3_adaptive_concurrency(...)` keeps the kwarg's own truthiness.
pub(super) const fn nonzero(value: Option<&OptionValue>) -> bool {
    match value {
        None | Some(OptionValue::Other(_)) => false,
        Some(OptionValue::Bool(flag)) => *flag,
        Some(OptionValue::Integer(integer)) => *integer != 0,
        Some(OptionValue::Text(text)) => !text.is_empty(),
        Some(OptionValue::Float(number)) => !matches!(number.classify(), FpCategory::Zero),
    }
}

fn legacy_flags<Config>(
    source: &Source<Config>,
    defaults: &Source<Config>,
    output: &mut BTreeSet<Feature>,
) where
    Config: Default,
{
    let flags = [
        (
            Feature::TeamPrefix,
            &source.use_team_prefix,
            &defaults.use_team_prefix,
        ),
        (
            Feature::KeyPrefix,
            &source.use_key_prefix,
            &defaults.use_key_prefix,
        ),
        (
            Feature::StripBase64,
            &source.strip_base64_files,
            &defaults.strip_base64_files,
        ),
        (
            Feature::VirtualHostedStyle,
            &source.use_virtual_hosted_style,
            &defaults.use_virtual_hosted_style,
        ),
    ];
    for (feature, configured, fallback) in flags {
        if configured.is_some_and(|value| value.0) || fallback.is_some_and(|value| value.0) {
            output.insert(feature);
        }
    }
}
