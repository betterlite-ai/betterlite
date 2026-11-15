use std::error::Error;
use std::fmt::Debug;

use crate::finops::focus::destinations::focus_gcs_destination::{
    FocusGcsDestination, GcsDeliveryError,
};
use crate::finops::focus::destinations::focus_s3_destination::{
    FocusS3Destination, S3DeliveryError,
};
use crate::finops::vantage::destination::VantageDestination;
use crate::finops::vantage::error::DeliveryError as VantageDeliveryError;
use crate::finops::vantage::sender::CsvSender;
use crate::focus_records::destinations::{FocusDestination, FocusTimeWindow};
use crate::gcs_mock::auth::VertexAuth;
use crate::gcs_mock::client::GcsHttpClient;
use crate::mavvrik_delivery::destination::FocusMavvrikDestination;
use crate::mavvrik_delivery::error::MavvrikDeliveryError;
use crate::mavvrik_delivery::transport::Transport;

pub enum Destination<GcsAuth, GcsHttp, CsvUpload, MavvrikHttp>
where
    MavvrikHttp: Transport,
{
    S3(FocusS3Destination),
    Gcs(FocusGcsDestination<GcsAuth, GcsHttp>),
    Vantage(VantageDestination<CsvUpload>),
    Mavvrik(FocusMavvrikDestination<MavvrikHttp>),
}

#[derive(Debug, thiserror::Error)]
pub enum DeliveryError<GcsAuth, GcsHttp, CsvFailure, MavvrikFailure>
where
    GcsAuth: VertexAuth + Debug + 'static,
    GcsHttp: GcsHttpClient + Debug + 'static,
    CsvFailure: Error + 'static,
    MavvrikFailure: Error + 'static,
{
    #[error("the S3 FOCUS upload failed")]
    S3(#[from] S3DeliveryError),
    #[error("the GCS FOCUS upload failed")]
    Gcs(#[source] GcsDeliveryError<GcsAuth, GcsHttp>),
    #[error("the Vantage FOCUS upload failed")]
    Vantage(#[source] VantageDeliveryError<CsvFailure>),
    #[error("the Mavvrik FOCUS upload failed")]
    Mavvrik(#[source] MavvrikDeliveryError<MavvrikFailure>),
}

impl<GcsAuth, GcsHttp, CsvUpload, MavvrikHttp> FocusDestination
    for Destination<GcsAuth, GcsHttp, CsvUpload, MavvrikHttp>
where
    GcsAuth: VertexAuth + Clone + Debug + 'static,
    GcsHttp: GcsHttpClient + Debug + 'static,
    CsvUpload: CsvSender,
    MavvrikHttp: Transport,
{
    type Error = DeliveryError<GcsAuth, GcsHttp, CsvUpload::Error, MavvrikHttp::Error>;

    async fn deliver<'delivery>(
        &'delivery self,
        content: &'delivery [u8],
        time_window: &'delivery FocusTimeWindow,
        filename: &'delivery str,
    ) -> Result<(), Self::Error> {
        match self {
            Self::S3(destination) => destination
                .deliver(content, time_window, filename)
                .await
                .map_err(DeliveryError::S3),
            Self::Gcs(destination) => destination
                .deliver(content, time_window, filename)
                .await
                .map_err(DeliveryError::Gcs),
            Self::Vantage(destination) => destination
                .deliver(content, time_window, filename)
                .await
                .map_err(DeliveryError::Vantage),
            Self::Mavvrik(destination) => destination
                .deliver(content, time_window, filename)
                .await
                .map_err(DeliveryError::Mavvrik),
        }
    }
}
