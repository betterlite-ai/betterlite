use std::collections::BTreeMap;
use std::env::consts::OS;
use std::fs;
use std::io;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use betterlite_azure::azure_arc_identity::{ArcHost, ArcPlatform};
use betterlite_azure::azure_client_secret::transport::Clock as TokenClock;
use betterlite_azure::azure_managed_identity::runtime::Scheduler;
use betterlite_azure::oidc_exchange::transport::Clock as OidcClock;
use tokio::runtime::Handle;
use tokio::time::sleep;

pub struct ProductionClock;

impl ProductionClock {
    fn unix_seconds() -> i64 {
        let elapsed = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or(Duration::ZERO)
            .as_secs();
        i64::try_from(elapsed).unwrap_or(i64::MAX)
    }
}

impl TokenClock for ProductionClock {
    fn now_unix_seconds(&self) -> i64 {
        Self::unix_seconds()
    }
}

impl OidcClock for ProductionClock {
    fn now_unix_seconds(&self) -> f64 {
        let elapsed = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or(Duration::ZERO);
        elapsed.as_secs_f64()
    }
}

pub struct ThreadScheduler {
    pub runtime: Handle,
}

impl Scheduler for ThreadScheduler {
    fn sleep(&self, delay: Duration) {
        self.runtime.block_on(sleep(delay));
    }
}

pub struct ProductionHost {
    pub environment: BTreeMap<String, String>,
}

impl ArcHost for ProductionHost {
    fn platform(&self) -> ArcPlatform {
        if cfg!(target_os = "linux") {
            ArcPlatform::Linux
        } else if cfg!(target_os = "windows") {
            ArcPlatform::Windows
        } else {
            ArcPlatform::Other(OS.to_owned())
        }
    }

    fn environment(&self, name: &str) -> Option<String> {
        self.environment.get(name).cloned()
    }

    fn program_data(&self) -> String {
        self.environment
            .get("ProgramData")
            .cloned()
            .unwrap_or_default()
    }

    fn program_files(&self) -> String {
        self.environment
            .get("ProgramFiles")
            .cloned()
            .unwrap_or_default()
    }

    fn file_size(&self, path: &str) -> io::Result<u64> {
        fs::metadata(path).map(|metadata| metadata.len())
    }

    fn read_text(&self, path: &str) -> io::Result<String> {
        fs::read_to_string(path)
    }

    fn now_unix_seconds(&self) -> i64 {
        ProductionClock::unix_seconds()
    }
}
