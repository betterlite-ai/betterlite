use std::error::Error as _;
use std::time::Instant;

use crate::database_failures::metrics::DbServiceEventSink;
use crate::key_user_writes::retry::WriteClock;

use super::error::{KeyReadError, READ_CALL_TYPE};
use super::ports::{
    Diagnostic, FailureHook, KeyFailureHook, KeyReadDatabase, KeyReadDiagnostics, PlanReconnect,
};
use super::reader::KeyReader;

pub(crate) const FAILURE_HEADING: &str =
    "LiteLLM Prisma Client Exception: Error with `get_data`. Args passed in: [redacted] ";

impl<Database, Reconnect, Sink> KeyReader<'_, Database, Reconnect, Sink>
where
    Database: KeyReadDatabase,
    Reconnect: PlanReconnect,
    Sink: DbServiceEventSink + 'static,
{
    // the source prints, logs a traceback and detaches failure_handler before the error leaves get_data
    pub(super) fn report_failure(&self, error: &KeyReadError, started: Instant) {
        let duration = self
            .services
            .clock
            .instant()
            .saturating_duration_since(started);
        let message = format!("{FAILURE_HEADING}{error}");
        self.services
            .diagnostics
            .emit(Diagnostic::AttemptFailure { message: &message });
        let traceback = failure_traceback(error, &message);
        self.services
            .diagnostics
            .emit(Diagnostic::FailureTraceback {
                message: &traceback,
            });
        let hook = FailureHook {
            call_type: READ_CALL_TYPE,
            class: error.class(),
            message,
            traceback,
            duration,
        };
        drop(self.services.tasks.spawn(self.services.hook.schedule(hook)));
    }
}

fn failure_traceback(error: &KeyReadError, message: &str) -> String {
    let mut rendered = message.to_owned();
    let mut cause = error.source();
    while let Some(source) = cause {
        rendered.push_str("\nCaused by: ");
        rendered.push_str(&source.to_string());
        cause = source.source();
    }
    rendered
}
