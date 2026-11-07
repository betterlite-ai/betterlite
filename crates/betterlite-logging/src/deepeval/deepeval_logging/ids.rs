use uuid::Uuid;

// uuid.uuid4() in _create_base_api_span and _create_trace_api, injected so a test or an oracle run can fix the ids.
pub trait UuidSource: Send + Sync {
    fn next_uuid(&self) -> String;
}

#[derive(Debug, Default)]
pub struct SystemUuids;

impl UuidSource for SystemUuids {
    fn next_uuid(&self) -> String {
        Uuid::new_v4().to_string()
    }
}
