use std::error::Error as StdError;

use async_trait::async_trait;

use crate::alert_dispatch::AlertLevel;

use super::record::{OutageRecord, RegionOutageRecord};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OutageDeployment {
    pub model: String,
    pub custom_llm_provider: Option<String>,
    pub region_name: Option<String>,
    pub vertex_location: Option<String>,
    pub aws_region_name: Option<String>,
    pub watsonx_region_name: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProviderResolution {
    pub model: String,
    pub provider: String,
}

pub trait OutageDeployments: Send + Sync {
    type Error: StdError + Send + Sync + 'static;

    fn deployment(&self, deployment_id: &str) -> Result<Option<OutageDeployment>, Self::Error>;
}

pub trait OutageProviders: Send + Sync {
    type Error: StdError + Send + Sync + 'static;

    fn get_llm_provider(&self, model: &str) -> Result<ProviderResolution, Self::Error>;
}

pub trait OutageApiBase: Send + Sync {
    type Error: StdError + Send + Sync + 'static;

    fn api_base(&self, model: &str, deployment_id: &str) -> Result<Option<String>, Self::Error>;
}

#[async_trait]
pub trait OutageStore: Send + Sync {
    type Error: StdError + Send + Sync + 'static;

    async fn get_outage_record(&self, key: &str) -> Result<Option<OutageRecord>, Self::Error>;

    async fn set_outage_record(
        &self,
        key: &str,
        record: &OutageRecord,
        ttl_seconds: Option<u32>,
    ) -> Result<(), Self::Error>;

    async fn get_region_outage_record(
        &self,
        key: &str,
    ) -> Result<Option<RegionOutageRecord>, Self::Error>;

    async fn set_region_outage_record(
        &self,
        key: &str,
        record: &RegionOutageRecord,
        ttl_seconds: Option<u32>,
    ) -> Result<(), Self::Error>;
}

#[async_trait]
pub trait OutageSender: Send + Sync {
    type Error: StdError + Send + Sync + 'static;

    async fn send_outage_alert(&self, message: &str, level: AlertLevel) -> Result<(), Self::Error>;
}

pub trait OutageClock: Send + Sync {
    fn now_unix_seconds(&self) -> f64;
}
