pub const MAX_LISTING_BYTES: usize = 1024 * 1024;
pub const MAX_LISTING_ENTRIES: usize = 4096;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ListingRefusal {
    TooLarge,
    Malformed,
    TooManyEntries,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DecodedPaths {
    pub paths: Vec<String>,
    pub refusal: Option<ListingRefusal>,
}

#[must_use]
pub fn decode_generated_paths(stdout: &str) -> DecodedPaths {
    if stdout.len() > MAX_LISTING_BYTES {
        return DecodedPaths {
            paths: Vec::new(),
            refusal: Some(ListingRefusal::TooLarge),
        };
    }
    let Ok(mut paths) = serde_json::from_str::<Vec<String>>(stdout.trim()) else {
        return DecodedPaths {
            paths: Vec::new(),
            refusal: Some(ListingRefusal::Malformed),
        };
    };
    if paths.len() > MAX_LISTING_ENTRIES {
        paths.truncate(MAX_LISTING_ENTRIES);
        return DecodedPaths {
            paths,
            refusal: Some(ListingRefusal::TooManyEntries),
        };
    }
    DecodedPaths {
        paths,
        refusal: None,
    }
}
