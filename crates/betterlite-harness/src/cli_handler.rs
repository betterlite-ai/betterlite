pub mod io;
mod setup;
mod tests;
mod turn_io;

use std::collections::VecDeque;
use std::future::{Future, ready};
use std::io as std_io;

use async_stream::try_stream;
use futures_core::Stream;

use turn_io::{TurnCursor, TurnProgress, next_turn_progress};

use crate::context::SessionContext;
use crate::errors::HarnessError;
use crate::events::Event;
use crate::handlers::BaseHarnessHandler;
use crate::payloads::decode_json_line;
use crate::sandbox::{Process as _, Sandbox, SandboxCommand};
use crate::transformation::{BaseCLIHarnessConfig, HarnessSessionSetup, TurnInput};

pub struct CLIHarnessHandler<C> {
    pub(crate) config: C,
    pub(crate) private_dir: Option<String>,
    pub(crate) setup: Option<HarnessSessionSetup>,
    pub(crate) native_id: Option<String>,
}

impl<C> CLIHarnessHandler<C>
where
    C: BaseCLIHarnessConfig,
{
    async fn spawn_turn<S>(
        &self,
        context: &SessionContext<S>,
        prompt: &str,
    ) -> Result<(S::Process, C::StreamState), HarnessError>
    where
        S: Sandbox,
        C: Sync,
    {
        let private_dir = self
            .private_dir
            .clone()
            .ok_or(HarnessError::SessionClosed)?;
        let setup = self.setup.clone().ok_or(HarnessError::SessionClosed)?;
        let native_id = self.native_id.clone();
        let request = self.config.transform_turn_request(
            context,
            TurnInput {
                setup: &setup,
                private_dir: &private_dir,
                prompt,
                native_session_id: native_id.as_deref(),
            },
        )?;
        let command = SandboxCommand {
            argv: request.argv,
            env: request.env,
            cwd: request.cwd,
        };
        let mut proc = context.sandbox.exec(&command).await?;
        if let Err(error) = io::send_stdin(&mut proc, &request.stdin).await {
            drop(proc.kill().await);
            return Err(error);
        }
        Ok((proc, self.config.create_stream_state()))
    }
}

impl<C> BaseHarnessHandler for CLIHarnessHandler<C>
where
    C: BaseCLIHarnessConfig + Send + Sync,
    C::StreamState: Send,
{
    type Config = C;

    fn config(&self) -> &C {
        &self.config
    }

    async fn start<S>(&mut self, context: &mut SessionContext<S>) -> Result<(), HarnessError>
    where
        S: Sandbox,
    {
        self.config.validate_environment(context)?;
        let binary = self.config.get_binary();
        if context.sandbox.which(binary).await?.is_none() {
            return Err(HarnessError::InstallFailed(std_io::Error::new(
                std_io::ErrorKind::NotFound,
                format!(
                    "`{binary}` was not found on PATH in the sandbox. Install it with: {}",
                    self.config.get_install_hint()
                ),
            )));
        }
        let private_dir = context.sandbox.tempdir().await?;
        let setup = self.config.transform_session_setup(context, &private_dir)?;
        setup::apply_session_setup(
            &context.sandbox,
            &private_dir,
            &setup,
            &context.settings.skills,
        )
        .await?;
        self.private_dir = Some(private_dir);
        self.setup = Some(setup);
        Ok(())
    }

    // async-stream's ? desugaring always calls Into::into to build the stream's error item
    #[expect(
        clippy::disallowed_methods,
        reason = "try_stream!'s ? desugaring always calls Into::into to build the yielded Err; ordinary ? elsewhere still uses From::from and stays checked"
    )]
    fn turn<S>(
        &mut self,
        context: &mut SessionContext<S>,
        prompt: &str,
    ) -> impl Stream<Item = Result<Event, HarnessError>> + Send
    where
        S: Sandbox,
    {
        let prompt_owned = prompt.to_owned();
        try_stream! {
            let (mut proc, mut state) = self.spawn_turn(context, &prompt_owned).await?;
            let mut tail = VecDeque::new();
            let mut cursor = TurnCursor::default();
            let outcome: Result<i32, HarnessError> = loop {
                match next_turn_progress(&mut proc, &mut cursor).await {
                    Ok(Some(TurnProgress::Stdout(raw))) => {
                        if let Some(line) = decode_json_line(&raw) {
                            for event in self.config.transform_stream_line(&line, &mut state) {
                                yield event;
                            }
                            if let Some(id) = self.config.get_native_session_id(&state) {
                                self.native_id = Some(id);
                            }
                        }
                    }
                    Ok(Some(TurnProgress::Stderr(raw))) => {
                        io::push_tail(&mut tail, io::decode_lossy(&raw));
                    }
                    Ok(None) => break proc.wait().await,
                    Err(error) => break Err(error),
                }
            };
            let exit_code = match outcome {
                Ok(code) => code,
                Err(error) => {
                    drop(proc.kill().await);
                    Err(error)?
                }
            };
            let stderr_tail = tail.into_iter().collect::<Vec<_>>();
            let response = self.config.transform_turn_response(context, &state, exit_code, &stderr_tail)?;
            context.outcome.final_text = response.final_text;
            context.outcome.output_json = response.output_json;
        }
    }

    // turn() already kills its own process on an early or errored exit
    fn stop<S>(
        &mut self,
        _: &mut SessionContext<S>,
    ) -> impl Future<Output = Result<(), HarnessError>> + Send
    where
        S: Sandbox,
    {
        ready(Ok(()))
    }

    fn native_session_id(&self) -> Option<String> {
        self.native_id.clone()
    }

    fn resume<S>(
        &mut self,
        _: &mut SessionContext<S>,
        native_session_id: &str,
    ) -> impl Future<Output = Result<(), HarnessError>> + Send
    where
        S: Sandbox,
    {
        self.native_id = Some(native_session_id.to_owned());
        ready(Ok(()))
    }
}
