use betterlite_azure::azure_default_chain::grant::{
    CertificateRecord, ClientSecretRecord, ManagedIdentityRecord, UsernamePasswordRecord,
    WorkloadRecord,
};
use secrecy::ExposeSecret as _;
use serde_json::{Value, json};

pub(super) fn secret_fields(record: &ClientSecretRecord) -> Value {
    json!({
        "tenant_id": record.tenant_id,
        "client_id": record.client_id,
        "client_secret": record.client_secret.expose_secret().to_owned(),
    })
}

pub(super) fn certificate_fields(record: &CertificateRecord) -> Value {
    json!({
        "tenant_id": record.tenant_id,
        "client_id": record.client_id,
        "certificate_path": record.certificate_path,
        "password": record
            .password
            .as_ref()
            .map(|secret| secret.expose_secret().to_owned()),
        "send_certificate_chain": record.send_certificate_chain,
    })
}

pub(super) fn password_fields(record: &UsernamePasswordRecord) -> Value {
    let mut fields = serde_json::Map::new();
    let tenant_id = json!(record.tenant_id);
    let client_id = json!(record.client_id);
    let username = json!(record.username);
    let password = json!(record.password.expose_secret());
    fields.insert("tenant_id".to_owned(), tenant_id);
    fields.insert("client_id".to_owned(), client_id);
    fields.insert("username".to_owned(), username);
    fields.insert("password".to_owned(), password);
    Value::Object(fields)
}

pub(super) fn workload_fields(record: &WorkloadRecord) -> Value {
    json!({
        "tenant_id": record.tenant_id,
        "client_id": record.client_id,
        "token_file": record.token_file,
    })
}

pub(super) fn managed_fields(record: &ManagedIdentityRecord) -> Value {
    let pairs: [(String, Value); 5] = [
        ("client_id".to_owned(), json!(record.client_id)),
        (
            "exclude_workload_identity".to_owned(),
            json!(record.exclude_workload_identity),
        ),
        ("imds_probe".to_owned(), json!(record.imds_probe)),
        (
            "within_credential_chain".to_owned(),
            json!(record.placement.within_credential_chain()),
        ),
        (
            "within_default_credential".to_owned(),
            json!(record.placement.within_default_credential()),
        ),
    ];
    Value::Object(pairs.into_iter().collect())
}
