use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecodedQuery {
    pub pairs: Vec<(String, String)>,
    pub last_value: BTreeMap<String, String>,
}

#[must_use]
pub fn decode_list_query(raw: &[u8]) -> DecodedQuery {
    let pairs: Vec<(String, String)> = form_urlencoded::parse(raw).into_owned().collect();
    let last_value = pairs.iter().cloned().collect();
    DecodedQuery { pairs, last_value }
}
