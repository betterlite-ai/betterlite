use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Clone, PartialEq, serde::Deserialize, serde::Serialize)]
#[serde(untagged)]
pub enum RequestRecord {
    Null,
    Bool(bool),
    Signed(i64),
    Unsigned(u64),
    Float(f64),
    String(String),
    Array(Vec<Self>),
    Object(BTreeMap<String, Self>),
}

impl RequestRecord {
    const fn is_null(&self) -> bool {
        let result = matches!(self, Self::Null);
        if result {
            return true;
        }
        false
    }

    const fn object(&self) -> Option<&BTreeMap<String, Self>> {
        if let Self::Object(fields) = self {
            Some(fields)
        } else {
            None
        }
    }

    const fn object_mut(&mut self) -> Option<&mut BTreeMap<String, Self>> {
        if let Self::Object(fields) = self {
            Some(fields)
        } else {
            None
        }
    }
}

pub struct RequestRecordProjection;

impl RequestRecordProjection {
    fn saved_span(fields: &mut BTreeMap<String, RequestRecord>) -> Option<RequestRecord> {
        let metadata_span = Self::placeholder(fields, "metadata");
        let internal_span = Self::placeholder(fields, "litellm_metadata");
        internal_span.or(metadata_span)
    }

    fn placeholder(
        fields: &mut BTreeMap<String, RequestRecord>,
        name: &str,
    ) -> Option<RequestRecord> {
        let bucket = fields.get_mut(name)?.object_mut()?;
        if !bucket.contains_key("litellm_parent_otel_span") {
            return None;
        }
        bucket.insert(
            "litellm_parent_otel_span".to_owned(),
            RequestRecord::String("placeholder".to_owned()),
        )
    }

    fn restore(fields: &mut BTreeMap<String, RequestRecord>, name: &str, span: &RequestRecord) {
        let Some(bucket) = fields.get_mut(name).and_then(RequestRecord::object_mut) else {
            return;
        };
        if bucket.contains_key("litellm_parent_otel_span") {
            bucket.insert("litellm_parent_otel_span".to_owned(), span.clone());
        }
    }

    #[must_use]
    pub fn safe_copy(mut record: RequestRecord, safe_memory_mode: bool) -> RequestRecord {
        if safe_memory_mode {
            return record;
        }
        let saved = record.object_mut().and_then(Self::saved_span);
        let copied = record.clone();
        if let (Some(fields), Some(span)) = (record.object_mut(), saved) {
            for bucket_name in ["metadata", "litellm_metadata"] {
                Self::restore(fields, bucket_name, &span);
            }
        }
        copied
    }

    #[must_use]
    pub fn independent_snapshot(record: &RequestRecord) -> RequestRecord {
        let snapshot = record.clone();
        if snapshot.is_null() {
            return snapshot;
        }
        snapshot
    }

    #[must_use]
    pub fn filter_parameters(record: RequestRecord, max_depth: i32) -> RequestRecord {
        if max_depth <= 0_i32 {
            return record;
        }
        Self::filter_value(record, max_depth)
    }

    fn filter_value(record: RequestRecord, depth: i32) -> RequestRecord {
        if depth <= 0_i32 {
            return record;
        }
        match record {
            RequestRecord::Object(fields) => RequestRecord::Object(
                fields
                    .into_iter()
                    .filter_map(|(key, value)| {
                        let filtered = Self::filter_value(value, depth.saturating_sub(1));
                        (!filtered.is_null()).then_some((key, filtered))
                    })
                    .collect(),
            ),
            RequestRecord::Array(values) => RequestRecord::Array(
                values
                    .into_iter()
                    .filter_map(|value| {
                        let filtered = Self::filter_value(value, depth.saturating_sub(1));
                        (!filtered.is_null()).then_some(filtered)
                    })
                    .collect(),
            ),
            RequestRecord::Null
            | RequestRecord::Bool(_)
            | RequestRecord::Signed(_)
            | RequestRecord::Unsigned(_)
            | RequestRecord::Float(_)
            | RequestRecord::String(_) => record,
        }
    }

    #[must_use]
    pub fn filter_internal(record: RequestRecord, additional: &BTreeSet<String>) -> RequestRecord {
        let RequestRecord::Object(fields) = record else {
            return record;
        };
        let internal = [
            "skip_mcp_handler",
            "mcp_handler_context",
            "_skip_mcp_handler",
        ];
        RequestRecord::Object(
            fields
                .into_iter()
                .filter(|(key, _)| !internal.contains(&key.as_str()) && !additional.contains(key))
                .collect(),
        )
    }
}

pub struct GatewayCacheAttribution;

impl GatewayCacheAttribution {
    fn metadata_bucket(
        fields: &mut BTreeMap<String, RequestRecord>,
    ) -> Option<&mut BTreeMap<String, RequestRecord>> {
        let name = if fields
            .get("litellm_metadata")
            .is_some_and(|value| value.object().is_some())
        {
            "litellm_metadata"
        } else {
            "metadata"
        };
        fields.get_mut(name).and_then(RequestRecord::object_mut)
    }

    pub fn record(record: &mut RequestRecord, added: i32, every_deployment: bool) {
        if added <= 0_i32 {
            return;
        }
        let Some(fields) = record.object_mut() else {
            return;
        };
        let marker = fields
            .get("model_info")
            .and_then(RequestRecord::object)
            .and_then(|info| info.get("id"))
            .cloned()
            .unwrap_or_else(|| RequestRecord::String(String::new()));
        let bucket = Self::metadata_bucket(fields);
        let Some(metadata_bucket) = bucket else {
            return;
        };
        if metadata_bucket.get("litellm_gateway_injected_cache")
            == Some(&RequestRecord::String(String::new()))
        {
            return;
        }
        let value = if every_deployment {
            RequestRecord::String(String::new())
        } else {
            marker
        };
        metadata_bucket.insert("litellm_gateway_injected_cache".to_owned(), value);
    }
}

#[path = "request_records/tests.rs"]
mod request_records_cases;
