use std::sync::{Arc, Mutex, PoisonError};

use betterlite_config::loader::EnvironmentLookup;
use reqwest::Client;

use crate::hook_initializers::secret_lookup::SecretLookup;
use crate::mcp_jwt_signer::JwtSigner;
use crate::mcp_jwt_signer_init::{CallbackRegistry, JwtSignerSlot};

// what the initializers take besides the params and the guardrail name
pub struct HookPorts {
    pub secrets: Arc<dyn SecretLookup + Send + Sync>,
    pub environment: Arc<dyn EnvironmentLookup>,
    pub router_present: bool,
    pub slot: Arc<JwtSignerSlot>,
    pub registered: Arc<Mutex<Vec<Arc<JwtSigner>>>>,
    pub client: Client,
}

impl HookPorts {
    #[must_use]
    pub fn callbacks(&self) -> HookCallbacks {
        HookCallbacks {
            environment: Arc::clone(&self.environment),
            registered: Arc::clone(&self.registered),
            client: self.client.clone(),
        }
    }
}

// LiteLLM's logging_callback_manager, which every folder initializer adds its instance to
pub struct HookCallbacks {
    environment: Arc<dyn EnvironmentLookup>,
    registered: Arc<Mutex<Vec<Arc<JwtSigner>>>>,
    client: Client,
}

impl CallbackRegistry for HookCallbacks {
    fn add_litellm_callback(&mut self, signer: Arc<JwtSigner>) {
        self.registered
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(signer);
    }

    fn environment(&self) -> &dyn EnvironmentLookup {
        self.environment.as_ref()
    }

    fn http_client(&self) -> Client {
        self.client.clone()
    }
}
