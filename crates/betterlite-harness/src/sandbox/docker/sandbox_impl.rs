use std::collections::BTreeMap;
use std::mem;
use std::sync::Arc;
use std::time::Duration;

use futures_util::future::join_all;

use super::error::DockerSandboxError;
use super::process::{self, DockerProcess};
use super::{DOCKER_HOST_ALIAS, DockerSandbox, MAX_HELPER_OUTPUT_BYTES, commands, files, snapshot};
use crate::errors::{CappedOperation, HarnessError};
use crate::sandbox::{CompletedRun, Sandbox, SandboxCommand};

impl Sandbox for DockerSandbox {
    type Process = DockerProcess;

    fn workdir(&self) -> &str {
        &self.workdir
    }

    fn is_container(&self) -> bool {
        true
    }

    async fn exec(&self, command: &SandboxCommand) -> Result<Self::Process, HarnessError> {
        if command.argv.is_empty() {
            return Err(HarnessError::from(DockerSandboxError::EmptyCommand));
        }
        let container_id = self.start().await?;
        let args = self.exec_args(
            &container_id,
            &command.argv,
            &command.env,
            command.cwd.as_deref(),
        );
        let client = process::start(&self.docker_program()?, &args)?;
        self.state
            .lock()
            .await
            .processes
            .push(Arc::clone(&client.child));
        Ok(client)
    }

    async fn run(
        &self,
        command: &SandboxCommand,
        timeout: Duration,
        max_output_bytes: usize,
    ) -> Result<CompletedRun, HarnessError> {
        let mut client = self.exec(command).await?;
        let result = process::collect(&mut client, command, timeout, max_output_bytes).await;
        self.state
            .lock()
            .await
            .processes
            .retain(|registered| !Arc::ptr_eq(registered, &client.child));
        result
    }

    async fn read(&self, path: &str, max_bytes: usize) -> Result<Vec<u8>, HarnessError> {
        files::read(self, path, max_bytes).await
    }

    async fn write(&self, path: &str, data: &[u8]) -> Result<(), HarnessError> {
        files::write(self, path, data).await
    }

    fn host_url(&self, port: u16) -> String {
        format!("http://{DOCKER_HOST_ALIAS}:{port}")
    }

    async fn which(&self, binary: &str) -> Result<Option<String>, HarnessError> {
        commands::which(self, binary).await
    }

    async fn snapshot(&self) -> Result<BTreeMap<String, String>, HarnessError> {
        snapshot::capture(self).await
    }

    async fn tempdir(&self) -> Result<String, HarnessError> {
        commands::tempdir(self).await
    }

    async fn close(&self) -> Result<(), HarnessError> {
        let (live_processes, stored_container) = {
            let mut state = self.state.lock().await;
            if state.closed {
                return Ok(());
            }
            state.closed = true;
            (mem::take(&mut state.processes), state.container_id.take())
        };
        drop(join_all(live_processes.iter().map(|process| process.kill())).await);
        if let Some(container_id) = stored_container {
            let args = ["rm".to_owned(), "-f".to_owned(), container_id];
            let mut client = process::start(&self.docker_program()?, &args)?;
            process::finish(
                &mut client,
                None,
                MAX_HELPER_OUTPUT_BYTES,
                CappedOperation::Read,
            )
            .await?;
        }
        Ok(())
    }
}
