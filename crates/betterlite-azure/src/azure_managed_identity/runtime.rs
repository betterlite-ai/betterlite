use std::fs;
use std::io;
use std::time::Duration;

use crate::azure_arc_identity::ArcTransport;
use crate::azure_certificate::assertion::IdSource;
use crate::azure_client_secret::transport::Clock;
use crate::azure_hosted_identity::request::HostedIo;
use crate::oidc_exchange::AzureOidcExchange;

use super::imds::ImdsTransport;

// the credential path is synchronous, so the caller supplies the scheduler its runtime tolerates
pub trait Scheduler: Send + Sync {
    fn sleep(&self, delay: Duration);
}

pub trait TokenFiles: Send + Sync {
    fn read(&self, path: &str) -> io::Result<Vec<u8>>;
}

#[derive(Debug, Default)]
pub struct FsTokenFiles;

impl TokenFiles for FsTokenFiles {
    fn read(&self, path: &str) -> io::Result<Vec<u8>> {
        fs::read(path)
    }
}

#[derive(Clone, Copy)]
pub struct ImdsIo<'io> {
    pub transport: &'io dyn ImdsTransport,
    pub scheduler: &'io dyn Scheduler,
    pub ids: &'io dyn IdSource,
    pub clock: &'io dyn Clock,
}

pub struct WorkloadIo<'io> {
    pub exchange: &'io AzureOidcExchange,
    pub files: &'io dyn TokenFiles,
}

pub struct ManagedIdentityIo<'io> {
    pub imds: ImdsIo<'io>,
    pub hosted: HostedIo<'io>,
    pub arc: &'io dyn ArcTransport,
    pub workload: WorkloadIo<'io>,
}
