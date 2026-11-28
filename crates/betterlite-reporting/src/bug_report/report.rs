use std::any::type_name_of_val;
use std::error::Error;
use std::panic::Location;

use crate::bug_report::environment::{Host, build_environment_report};
use crate::bug_report::frames::litellm_frames;
use crate::bug_report::providers::KNOWN_PROVIDERS;
use crate::bug_report::record::{BugReport, Surface};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CallContext {
    pub surface: Surface,
    pub call_type: Option<String>,
    pub custom_llm_provider: Option<String>,
    pub stream: Option<bool>,
    pub config_lines: Vec<String>,
}

// Python's type(exc).__name__ keeps the class name alone, without its module or generic arguments
#[must_use]
pub fn exception_type_name<E>(exception: &E) -> &'static str
where
    E: Error,
{
    let full_name = type_name_of_val(exception);
    let path = full_name
        .split_once('<')
        .map_or(full_name, |(head, _)| head);
    path.rsplit_once("::").map_or(path, |(_, name)| name)
}

// The location is the one captured where the error was created, as Python keeps it on the exception
#[must_use]
pub fn build_bug_report(
    exception_type: &str,
    location: &Location<'_>,
    context: CallContext,
    host: Host<'_>,
) -> BugReport {
    BugReport {
        environment: build_environment_report(context.surface, host, context.config_lines),
        exception_type: exception_type.to_owned(),
        litellm_frames: litellm_frames(location.file(), location.line()),
        call_type: context.call_type,
        custom_llm_provider: context.custom_llm_provider.filter(|provider| {
            KNOWN_PROVIDERS
                .split_ascii_whitespace()
                .any(|known| known == provider)
        }),
        stream: context.stream,
    }
}
