use std::sync::{Arc, Mutex};

use async_trait::async_trait;

use crate::engine::Engine;
use crate::engine::selection::{Kind, SelectionError, Selector};

#[async_trait]
pub trait DefaultEngine: Send + Sync {
    async fn load_default(&self) -> Result<Engine, SelectionError>;
}

#[async_trait]
impl DefaultEngine for Selector {
    async fn load_default(&self) -> Result<Engine, SelectionError> {
        self.load_kind(Kind::OpenAi).await
    }
}

// main.py's _get_encoding: the module encoding is built on the first access and the same engine answers after it
pub struct DefaultEncoding {
    loader: Arc<dyn DefaultEngine>,
    cached: Mutex<Option<Engine>>,
}

impl DefaultEncoding {
    #[must_use]
    pub fn new(loader: Arc<dyn DefaultEngine>) -> Self {
        Self {
            loader,
            cached: Mutex::new(None),
        }
    }

    pub async fn get(&self) -> Result<Engine, SelectionError> {
        if let Some(found) = self.cached.lock().ok().and_then(|cache| cache.clone()) {
            return Ok(found);
        }
        let engine = self.loader.load_default().await?;
        if let Ok(mut cache) = self.cached.lock() {
            *cache = Some(engine.clone());
        }
        Ok(engine)
    }
}
