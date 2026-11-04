#![cfg(test)]

use std::collections::BTreeMap;

use crate::sections::Section;
use crate::sections::huggingface_hub::Raw;

#[test]
fn settings_follow_huggingface_hub() {
    let settings = Raw::default().convert(&BTreeMap::new()).unwrap();
    assert_eq!(settings.endpoint, "https://huggingface.co");
    assert_eq!(settings.etag_timeout, 10);
    assert_eq!(settings.download_timeout, 10);
    assert_eq!(settings.hub_cache, settings.hf_home.join("hub"));
}
