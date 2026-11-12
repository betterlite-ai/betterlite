mod edit;
mod generation;
mod variation;

use std::sync::Mutex;
use std::sync::atomic::AtomicUsize;

use betterlite_openai::api::image_contracts::records::{ImageFile, ImageHeaders, ImageParams};

pub(super) use generation::GenerationFixture;
pub(super) use variation::VariationFixture;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Seen {
    pub(super) model: String,
    pub(super) prompt: Option<String>,
    pub(super) image: Option<ImageFile>,
    pub(super) options: ImageParams,
    pub(super) headers: ImageHeaders,
}

pub(super) struct EditFixture {
    pub(super) calls: AtomicUsize,
    pub(super) seen: Mutex<Vec<Seen>>,
    pub(super) raise_status: Option<u16>,
}
