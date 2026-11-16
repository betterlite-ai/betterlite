use super::super::Failure;
use super::super::session::Session;
use super::super::version::print_version;

pub(crate) fn run(session: &mut Session<'_>) -> Result<(), Failure> {
    print_version(
        &session.context,
        &session.client,
        &mut session.streams.stdout,
    )
}
