use std::fmt::Debug;
use std::sync::Arc;

use betterlite_config::loader::EnvironmentLookup;

use crate::finops::focus::destinations::focus_gcs_destination::FocusGcsDestination;
use crate::finops::focus::destinations::focus_s3_destination::{FocusS3Destination, S3Connection};
use crate::finops::vantage::destination::VantageDestination;
use crate::finops::vantage::sender::CsvSender;
use crate::focus_records::destinations::FocusDestination;
use crate::gcs_client::gcs_storage_auth::GcsBucketBase;
use crate::gcs_mock::auth::VertexAuth;
use crate::gcs_mock::client::GcsHttpClient;
use crate::mavvrik_delivery::destination::FocusMavvrikDestination;
use crate::mavvrik_delivery::transport::Transport;
use crate::s3_transport::signing::Clock;
use crate::s3_transport::transport::S3HttpClient;

pub mod destination;
pub mod error;
pub mod provider;
pub mod resolve;

mod tests;

use self::destination::Destination;
use self::error::FactoryError;
use self::provider::Provider;
use self::resolve::DestinationOverrides;

pub trait DestinationProvider {
    type Destination: FocusDestination;

    fn create(
        self,
        provider: &str,
        prefix: &str,
        overrides: &DestinationOverrides,
    ) -> Result<Self::Destination, FactoryError>;
}

pub struct FocusDestinationFactory<GcsAuth, GcsHttp, CsvUpload, MavvrikHttp> {
    pub environment: Arc<dyn EnvironmentLookup + Send + Sync>,
    pub s3_http: Arc<dyn S3HttpClient>,
    pub s3_clock: Arc<dyn Clock>,
    pub gcs: GcsBucketBase<GcsAuth, GcsHttp>,
    pub vantage: CsvUpload,
    pub mavvrik: MavvrikHttp,
}

impl<GcsAuth, GcsHttp, CsvUpload, MavvrikHttp> DestinationProvider
    for FocusDestinationFactory<GcsAuth, GcsHttp, CsvUpload, MavvrikHttp>
where
    GcsAuth: VertexAuth + Clone + Debug + 'static,
    GcsHttp: GcsHttpClient + Debug + 'static,
    CsvUpload: CsvSender,
    MavvrikHttp: Transport,
{
    type Destination = Destination<GcsAuth, GcsHttp, CsvUpload, MavvrikHttp>;

    fn create(
        self,
        provider: &str,
        prefix: &str,
        overrides: &DestinationOverrides,
    ) -> Result<Self::Destination, FactoryError> {
        let sources = resolve::Sources {
            overrides,
            environment: self.environment.as_ref(),
        };
        match Provider::parse(provider)? {
            Provider::S3 => {
                let settings = resolve::s3(&sources)?;
                let connection = S3Connection {
                    http: self.s3_http,
                    environment: self.environment,
                    clock: self.s3_clock,
                };
                FocusS3Destination::new(connection, prefix, settings)
                    .map(Destination::S3)
                    .map_err(FactoryError::S3Destination)
            }
            Provider::Vantage => {
                let settings = resolve::vantage(&sources)?;
                VantageDestination::new(self.vantage, settings)
                    .map(Destination::Vantage)
                    .map_err(FactoryError::VantageDestination)
            }
            Provider::Gcs => {
                let settings = resolve::gcs(&sources)?;
                FocusGcsDestination::new(self.gcs, prefix, settings)
                    .map(Destination::Gcs)
                    .map_err(FactoryError::GcsDestination)
            }
            Provider::Mavvrik => {
                let settings = resolve::mavvrik(&sources);
                FocusMavvrikDestination::new(self.mavvrik, settings)
                    .map(Destination::Mavvrik)
                    .map_err(FactoryError::MavvrikDestination)
            }
        }
    }
}
