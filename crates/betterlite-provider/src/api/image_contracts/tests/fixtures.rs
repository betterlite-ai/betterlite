mod edit;
mod generation;
mod variation;

use std::sync::Mutex;
use std::sync::atomic::AtomicUsize;

use betterlite_openai::api::image_contracts::records::{ImageFile, ImageHeaders, ImageParams};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Seen {
    pub(super) model: String,
    pub(super) prompt: Option<String>,
    pub(super) image: Option<ImageFile>,
    pub(super) options: ImageParams,
    pub(super) headers: ImageHeaders,
    pub(super) litellm_params: ImageParams,
}

#[derive(Default)]
pub(super) struct Fixture {
    pub(super) calls: AtomicUsize,
    pub(super) seen: Mutex<Vec<Seen>>,
    pub(super) raise_status: Option<u16>,
}

impl Fixture {
    pub(super) fn raising(status: u16) -> Self {
        Self {
            calls: AtomicUsize::new(0),
            seen: Mutex::new(Vec::new()),
            raise_status: Some(status),
        }
    }

    pub(super) fn seen(&self) -> Vec<Seen> {
        self.seen.lock().expect("seen").clone()
    }
}
