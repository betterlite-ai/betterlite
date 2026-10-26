use std::borrow::Cow;
use std::collections::{BTreeMap, BTreeSet};

pub mod record;
use record::{HandleKind, RequestRecord};

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum RequestRecordError {
    #[error("{bucket} must be an object when it contains a parent span")]
    NonObjectMetadata { bucket: String },
}

pub struct RequestRecordProjection;

impl RequestRecordProjection {
    fn sanitize_bucket(
        fields: &mut BTreeMap<String, RequestRecord>,
        name: &str,
    ) -> Result<Option<RequestRecord>, RequestRecordError> {
        let Some(value) = fields.get_mut(name) else {
            return Ok(None);
        };
        let Some(bucket) = value.object_mut() else {
            match value {
                RequestRecord::String(text) if text.contains("litellm_parent_otel_span") => {
                    return Err(RequestRecordError::NonObjectMetadata {
                        bucket: name.to_owned(),
                    });
                }
                RequestRecord::Null
                | RequestRecord::Bool(_)
                | RequestRecord::Signed(_)
                | RequestRecord::Unsigned(_)
                | RequestRecord::Decimal(_)
                | RequestRecord::Handle(_) => {
                    return Err(RequestRecordError::NonObjectMetadata {
                        bucket: name.to_owned(),
                    });
                }
                RequestRecord::Array(_) => return Ok(None),
                RequestRecord::String(_) | RequestRecord::Object(_) => {
                    return Ok(None);
                }
            }
        };
        let Some(span) = bucket.remove("litellm_parent_otel_span") else {
            return Ok(None);
        };
        bucket.insert(
            "litellm_parent_otel_span".to_owned(),
            RequestRecord::String("placeholder".to_owned()),
        );
        Ok(Some(span))
    }

    fn restore(fields: &mut BTreeMap<String, RequestRecord>, name: &str, span: &RequestRecord) {
        let Some(bucket) = fields.get_mut(name).and_then(RequestRecord::object_mut) else {
            return;
        };
        if bucket.contains_key("litellm_parent_otel_span") {
            bucket.insert("litellm_parent_otel_span".to_owned(), span.clone());
        }
    }

    fn restore_all(record: &mut RequestRecord, span: &RequestRecord) {
        if matches!(span, RequestRecord::Null) {
            return;
        }
        let Some(fields) = record.object_mut() else {
            return;
        };
        Self::restore(fields, "metadata", span);
        Self::restore(fields, "litellm_metadata", span);
    }

    pub fn safe_copy(
        record: &mut RequestRecord,
        safe_memory_mode: bool,
    ) -> Result<Cow<'_, RequestRecord>, RequestRecordError> {
        if safe_memory_mode {
            return Ok(Cow::Borrowed(record));
        }
        let saved_metadata = record
            .object_mut()
            .map(|fields| Self::sanitize_bucket(fields, "metadata"))
            .transpose()?
            .flatten();
        let saved_internal = record
            .object_mut()
            .map(|fields| Self::sanitize_bucket(fields, "litellm_metadata"))
            .transpose()?
            .flatten();
        let copied = record.clone();
        if let Some(span) = saved_internal.or(saved_metadata) {
            Self::restore_all(record, &span);
        }
        Ok(Cow::Owned(copied))
    }

    #[must_use]
    pub fn independent_snapshot(record: &RequestRecord) -> RequestRecord {
        let Some(fields) = record.object() else {
            return record.clone();
        };
        let mut snapshot = BTreeMap::new();
        for (key, value) in fields {
            let copied = if matches!(key.as_str(), "metadata" | "litellm_metadata")
                && let Some(bucket) = value.object()
            {
                RequestRecord::Object(Self::snapshot_bucket(bucket))
            } else {
                value.clone()
            };
            snapshot.insert(key.clone(), copied);
        }
        RequestRecord::Object(snapshot)
    }

    fn snapshot_bucket(
        bucket: &BTreeMap<String, RequestRecord>,
    ) -> BTreeMap<String, RequestRecord> {
        let mut clean = bucket.clone();
        if let Some(original) = clean.get("litellm_parent_otel_span").cloned() {
            clean.insert(
                "litellm_parent_otel_span".to_owned(),
                RequestRecord::String("placeholder".to_owned()),
            );
            clean.insert("litellm_parent_otel_span".to_owned(), original);
        }
        clean
    }

    #[must_use]
    pub fn filter_parameters(record: RequestRecord, max_depth: i32) -> RequestRecord {
        if max_depth <= 0_i32 {
            return record;
        }
        Self::filter_value(record, max_depth)
    }

    fn filter_value(record: RequestRecord, depth: i32) -> RequestRecord {
        if let RequestRecord::Handle(handle) = &record {
            if matches!(handle.kind, HandleKind::Exception | HandleKind::Callable) {
                return RequestRecord::Null;
            }
            if depth <= 0_i32 {
                return record;
            }
            if matches!(
                handle.kind,
                HandleKind::Logging | HandleKind::LiteLLMLoggingObj | HandleKind::Router
            ) {
                return RequestRecord::Null;
            }
        }
        if depth <= 0_i32 {
            return record;
        }
        match record {
            RequestRecord::Object(fields) => Self::filter_object(fields, depth),
            RequestRecord::Array(values) => Self::filter_array(values, depth),
            RequestRecord::Null
            | RequestRecord::Bool(_)
            | RequestRecord::Signed(_)
            | RequestRecord::Unsigned(_)
            | RequestRecord::Decimal(_)
            | RequestRecord::String(_)
            | RequestRecord::Handle(_) => record,
        }
    }

    fn filter_object(fields: BTreeMap<String, RequestRecord>, depth: i32) -> RequestRecord {
        let filtered = fields
            .into_iter()
            .filter_map(|(key, value)| Self::filter_pair(key, value, depth))
            .collect();
        RequestRecord::Object(filtered)
    }

    fn filter_array(values: Vec<RequestRecord>, depth: i32) -> RequestRecord {
        let filtered = values
            .into_iter()
            .filter_map(|value| Self::filter_item(value, depth))
            .collect();
        RequestRecord::Array(filtered)
    }

    fn filter_pair(
        key: String,
        value: RequestRecord,
        depth: i32,
    ) -> Option<(String, RequestRecord)> {
        let filtered = Self::filter_item(value, depth);
        filtered.map(|item| (key, item))
    }

    fn filter_item(value: RequestRecord, depth: i32) -> Option<RequestRecord> {
        let filtered = Self::filter_value(value, depth.saturating_sub(1));
        (!matches!(filtered, RequestRecord::Null)).then_some(filtered)
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
    pub fn record(record: &mut RequestRecord, added: i32, every_deployment: bool) {
        if added <= 0_i32 {
            return;
        }
        let Some(fields) = record.object_mut() else {
            return;
        };
        let selected_bucket = ["litellm_metadata", "metadata"]
            .into_iter()
            .find(|name| fields.get(*name).and_then(RequestRecord::object).is_some());
        let Some(bucket_name) = selected_bucket else {
            return;
        };
        let marker = fields
            .get("model_info")
            .and_then(RequestRecord::object)
            .and_then(|info| info.get("id"))
            .cloned()
            .unwrap_or(RequestRecord::String(String::new()));
        let Some(bucket) = fields
            .get_mut(bucket_name)
            .and_then(RequestRecord::object_mut)
        else {
            return;
        };
        if bucket.get("litellm_gateway_injected_cache")
            == Some(&RequestRecord::String(String::new()))
        {
            return;
        }
        bucket.insert(
            "litellm_gateway_injected_cache".to_owned(),
            if every_deployment {
                RequestRecord::String(String::new())
            } else {
                marker
            },
        );
    }
}

#[path = "request_records/tests.rs"]
mod request_records_cases;
