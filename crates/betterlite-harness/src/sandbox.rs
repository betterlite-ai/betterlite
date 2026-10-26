pub mod local;

use std::collections::BTreeMap;
use std::future::Future;
use std::time::Duration;

use secrecy::SecretString;
use tokio::io::{AsyncRead, AsyncWrite};

use crate::errors::HarnessError;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CompletedRun {
    pub stdout: String,
    pub stderr: String,
    pub exit_code: i32,
}

#[derive(Clone, Debug, Default)]
pub struct SandboxCommand {
    pub argv: Vec<String>,
    pub env: BTreeMap<String, SecretString>,
    // absent means the sandbox workdir
    pub cwd: Option<String>,
}

pub trait Process: Send {
    type Stdin: AsyncWrite + Send + Unpin;
    type Stdout: AsyncRead + Send + Unpin;
    type Stderr: AsyncRead + Send + Unpin;

    fn stdin(&mut self) -> Option<&mut Self::Stdin>;
    fn stdout(&mut self) -> &mut Self::Stdout;
    fn stderr(&mut self) -> &mut Self::Stderr;
    fn wait(&mut self) -> impl Future<Output = Result<i32, HarnessError>> + Send;
    fn kill(&mut self) -> impl Future<Output = Result<(), HarnessError>> + Send;
}

pub trait Sandbox: Send + Sync {
    type Process: Process;

    fn workdir(&self) -> &str;
    fn exec(
        &self,
        command: &SandboxCommand,
    ) -> impl Future<Output = Result<Self::Process, HarnessError>> + Send;
    fn run(
        &self,
        command: &SandboxCommand,
        timeout: Duration,
        max_output_bytes: usize,
    ) -> impl Future<Output = Result<CompletedRun, HarnessError>> + Send;
    fn read(
        &self,
        path: &str,
        max_bytes: usize,
    ) -> impl Future<Output = Result<Vec<u8>, HarnessError>> + Send;
    fn write(
        &self,
        path: &str,
        data: &[u8],
    ) -> impl Future<Output = Result<(), HarnessError>> + Send;
    fn host_url(&self, port: u16) -> String;
    fn which(
        &self,
        binary: &str,
    ) -> impl Future<Output = Result<Option<String>, HarnessError>> + Send;
    fn snapshot(
        &self,
    ) -> impl Future<Output = Result<BTreeMap<String, String>, HarnessError>> + Send;
    fn tempdir(&self) -> impl Future<Output = Result<String, HarnessError>> + Send;
    fn close(&self) -> impl Future<Output = Result<(), HarnessError>> + Send;
}
