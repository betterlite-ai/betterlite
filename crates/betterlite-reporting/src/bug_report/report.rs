use std::any::type_name_of_val;
use std::error::Error;

use backtrace::Backtrace;

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

// The backtrace is the one captured where the error was created, as Python keeps it on the exception
#[must_use]
pub fn build_bug_report<E>(
    exception: &E,
    backtrace: &Backtrace,
    context: CallContext,
    host: Host<'_>,
) -> BugReport
where
    E: Error,
{
    let full_name = type_name_of_val(exception);
    let path = full_name
        .split_once('<')
        .map_or(full_name, |(head, _)| head);
    let exception_type = path.rsplit_once("::").map_or(path, |(_, name)| name);
    BugReport {
        environment: build_environment_report(context.surface, host, context.config_lines),
        exception_type: exception_type.to_owned(),
        litellm_frames: litellm_frames(backtrace),
        call_type: context.call_type,
        custom_llm_provider: context.custom_llm_provider.filter(|provider| {
            KNOWN_PROVIDERS
                .split_ascii_whitespace()
                .any(|known| known == provider)
        }),
        stream: context.stream,
    }
}
