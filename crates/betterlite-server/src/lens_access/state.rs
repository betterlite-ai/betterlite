use std::convert::Infallible;
use std::future::{Future, ready};
use std::sync::Arc;

use betterlite_db::lens_repository::database::Database;
use betterlite_db::lens_repository::repository::LensRepository;
use betterlite_lens::sources::parameters::Parameters;
use betterlite_lens::sources::storage::Storage;
use betterlite_lens::sources::storage::{ContentParameters, EvidenceParameters, SampleParameters};
use chrono::{DateTime, Utc};

use crate::lens_billing::key_validation::KeyResolver;

// where the routes read the current time: the system clock in production, a fixed instant in tests
pub(crate) trait Clock: Send + Sync {
    fn now(&self) -> DateTime<Utc>;
}

// uuid v4 text for new ids, and the operating system's random bytes for the worker token
pub(crate) trait IdSource: Send + Sync {
    fn new_id(&self) -> String;
    fn random_bytes(&self, count: usize) -> Vec<u8>;
}

// B844's get_model_names over the configured router, absent from the state when none is configured
pub(crate) trait ModelNames: Send + Sync {
    fn get_model_names(&self, team_id: Option<&str>) -> Vec<String>;
}

// the database and storage bounds every administration handler repeats, named once per trait
pub trait LensDatabase: Database + 'static {}
impl<T: Database + 'static> LensDatabase for T {}

pub trait LensStorage: Storage + Clone + Send + Sync + 'static {}
impl<T: Storage + Clone + Send + Sync + 'static> LensStorage for T {}

// the ports the administration routes and the worker routes read, built once by the mount
pub struct LensRoutesState<Db, Store = NoStorage> {
    pub(crate) repository: Option<Arc<LensRepository<Db>>>,
    pub(crate) storage: Option<Store>,
    pub(crate) models: Option<Arc<dyn ModelNames>>,
    pub(crate) keys: Arc<dyn KeyResolver>,
    pub(crate) clock: Arc<dyn Clock>,
    pub(crate) ids: Arc<dyn IdSource>,
}

impl<Db, Store: Clone> Clone for LensRoutesState<Db, Store> {
    fn clone(&self) -> Self {
        Self {
            repository: self.repository.clone(),
            storage: self.storage.clone(),
            models: self.models.clone(),
            keys: Arc::clone(&self.keys),
            clock: Arc::clone(&self.clock),
            ids: Arc::clone(&self.ids),
        }
    }
}

// the storage type a LensRoutesState names while no tracing store is connected
#[derive(Clone, Copy, Debug)]
pub struct NoStorage;

impl Storage for NoStorage {
    type Error = Infallible;

    fn lens_availability(
        &self,
        _parameters: Parameters,
    ) -> impl Future<Output = Result<Vec<u8>, Self::Error>> + Send {
        ready(Ok(Vec::new()))
    }

    fn lens_agents(
        &self,
        _parameters: Parameters,
    ) -> impl Future<Output = Result<Vec<u8>, Self::Error>> + Send {
        ready(Ok(Vec::new()))
    }

    fn lens_sample(
        &self,
        _parameters: SampleParameters,
    ) -> impl Future<Output = Result<Vec<u8>, Self::Error>> + Send {
        ready(Ok(Vec::new()))
    }

    fn lens_content(
        &self,
        _parameters: ContentParameters,
    ) -> impl Future<Output = Result<Vec<u8>, Self::Error>> + Send {
        ready(Ok(Vec::new()))
    }

    fn lens_evidence(
        &self,
        _parameters: EvidenceParameters,
    ) -> impl Future<Output = Result<Vec<u8>, Self::Error>> + Send {
        ready(Ok(Vec::new()))
    }
}
