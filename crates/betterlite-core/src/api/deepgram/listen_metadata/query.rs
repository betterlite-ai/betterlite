use std::collections::BTreeMap;

use url::form_urlencoded;

pub const DEFAULT_MODEL: &str = "nova-3";

const SINGLE_VALUED: [&str; 2] = ["model", "language"];
const CALLBACK_PARAMS: [&str; 2] = ["callback", "callback_method"];

// a listen query in wire order, duplicates kept, values percent-decoded like httpx.QueryParams
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ListenQuery {
    pub(crate) pairs: Vec<(String, String)>,
}

#[must_use]
pub fn decode_listen_query(query: &[u8]) -> ListenQuery {
    let mut decoded = ListenQuery::default();
    for (name, value) in form_urlencoded::parse(query) {
        decoded.pairs.push((name.into_owned(), value.into_owned()));
    }
    decoded
}

impl ListenQuery {
    #[must_use]
    pub fn first(&self, name: &str) -> Option<&str> {
        self.pairs
            .iter()
            .find(|(pair, _)| pair == name)
            .map(|(_, value)| value.as_str())
    }

    #[must_use]
    pub fn values(&self, name: &str) -> Vec<&str> {
        let mut found = Vec::new();
        for (pair, value) in &self.pairs {
            if pair == name {
                found.push(value.as_str());
            }
        }
        found
    }

    // httpx.QueryParams groups repeated keys at the first one's place; a second model or language is dropped
    #[must_use]
    pub fn first_occurrences(&self) -> Self {
        let mut order = Vec::new();
        let mut grouped: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
        for (name, value) in &self.pairs {
            grouped
                .entry(name.as_str())
                .or_insert_with(|| {
                    order.push(name.as_str());
                    Vec::new()
                })
                .push(value.as_str());
        }
        let mut kept = Self::default();
        for name in order {
            let limit = if SINGLE_VALUED.contains(&name) {
                1
            } else {
                usize::MAX
            };
            for value in grouped.get(name).into_iter().flatten().take(limit) {
                kept.pairs.push((name.to_owned(), (*value).to_owned()));
            }
        }
        kept
    }

    #[must_use]
    pub fn requested_model(&self) -> &str {
        let model = self.first("model").unwrap_or_default();
        if model.is_empty() {
            DEFAULT_MODEL
        } else {
            model
        }
    }

    #[must_use]
    pub fn callback_params(&self) -> Vec<&'static str> {
        let mut present = Vec::new();
        for name in CALLBACK_PARAMS {
            if self.pairs.iter().any(|(pair, _)| pair == name) {
                present.push(name);
            }
        }
        present
    }

    // the declared channels value is a count only when every byte is an ASCII digit and the number is positive
    #[must_use]
    pub fn declared_channels(&self) -> Option<u64> {
        let channels = self.first("channels")?;
        if channels.is_empty() || !channels.bytes().all(|byte| byte.is_ascii_digit()) {
            return None;
        }
        channels.parse::<u64>().ok().filter(|count| *count >= 1)
    }

    // parse_qs drops a pair whose value decodes empty, while httpx.QueryParams keeps it
    #[must_use]
    pub fn without_blank_values(&self) -> Self {
        let mut kept = Self::default();
        for (name, value) in &self.pairs {
            if !value.is_empty() {
                kept.pairs.push((name.clone(), value.clone()));
            }
        }
        kept
    }

    #[must_use]
    pub fn encode(&self) -> String {
        let mut serializer = form_urlencoded::Serializer::new(String::new());
        for (name, value) in &self.pairs {
            serializer.append_pair(name, value);
        }
        // Python's quote_plus leaves ~ alone and encodes *, the reverse of form_urlencoded
        serializer.finish().replace('*', "%2A").replace("%7E", "~")
    }
}
