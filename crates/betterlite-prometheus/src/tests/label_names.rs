use crate::label_names_catalog::UserApiKeyLabelName;
#[test]
fn exposes_domains() {
    assert_eq!(UserApiKeyLabelName::ALL.len(), 33);
    assert_eq!(UserApiKeyLabelName::HashedApiKey.as_str(), "hashed_api_key");
}
