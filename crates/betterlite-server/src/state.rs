use std::sync::Arc;

use secrecy::SecretString;

use crate::auth::auth_identity::AuthBackend;

pub struct AppState {
    pub master_key: Option<SecretString>,
    pub auth_backends: Vec<AuthBackend>,
}

impl AppState {
    #[must_use]
    pub fn new(master_key: Option<SecretString>) -> Arc<Self> {
        Arc::new(Self {
            master_key,
            auth_backends: vec![AuthBackend::MasterKey],
        })
    }
}
