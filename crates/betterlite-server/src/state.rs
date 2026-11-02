use secrecy::SecretString;

use crate::auth::auth_identity::AuthBackend;

pub struct AppState {
    pub master_key: Option<SecretString>,
    pub auth_backends: Vec<AuthBackend>,
}
