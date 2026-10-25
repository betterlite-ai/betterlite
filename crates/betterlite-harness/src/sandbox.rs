use std::collections::BTreeMap;
use std::future::Future;
use std::pin::Pin;

pub struct CompletedRun {
    pub stdout: String,
    pub stderr: String,
    pub exit_code: i32,
}
pub trait Process: Send {
    fn wait(
        &mut self,
    ) -> Pin<Box<dyn Future<Output = Result<i32, crate::errors::SandboxError>> + Send + '_>>;
    fn kill(
        &mut self,
    ) -> Pin<Box<dyn Future<Output = Result<(), crate::errors::SandboxError>> + Send + '_>>;
}
pub trait Sandbox: Send {
    fn workdir(&self) -> &str;
    fn exec(
        &self,
        command: Vec<String>,
        env: BTreeMap<String, String>,
        cwd: Option<String>,
    ) -> Pin<
        Box<dyn Future<Output = Result<Box<dyn Process>, crate::errors::SandboxError>> + Send + '_>,
    >;
    fn run(
        &self,
        command: Vec<String>,
        env: BTreeMap<String, String>,
        cwd: Option<String>,
        timeout_seconds: Option<f64>,
    ) -> Pin<Box<dyn Future<Output = Result<CompletedRun, crate::errors::SandboxError>> + Send + '_>>;
    fn read(
        &self,
        path: String,
    ) -> Pin<Box<dyn Future<Output = Result<Vec<u8>, crate::errors::SandboxError>> + Send + '_>>;
    fn write(
        &self,
        path: String,
        data: Vec<u8>,
    ) -> Pin<Box<dyn Future<Output = Result<(), crate::errors::SandboxError>> + Send + '_>>;
    fn host_url(&self, port: u16) -> String;
    fn which(
        &self,
        binary: String,
    ) -> Pin<
        Box<dyn Future<Output = Result<Option<String>, crate::errors::SandboxError>> + Send + '_>,
    >;
    fn snapshot(
        &self,
    ) -> Pin<
        Box<
            dyn Future<Output = Result<BTreeMap<String, String>, crate::errors::SandboxError>>
                + Send
                + '_,
        >,
    >;
    fn tempdir(
        &self,
    ) -> Pin<Box<dyn Future<Output = Result<String, crate::errors::SandboxError>> + Send + '_>>;
    fn close(
        &mut self,
    ) -> Pin<Box<dyn Future<Output = Result<(), crate::errors::SandboxError>> + Send + '_>>;
}
