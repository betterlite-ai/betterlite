use chrono::{DateTime, Utc};
use secrecy::ExposeSecret as _;

use crate::assume_role::expiry::{StsResponse, extract_credentials_and_ttl};

pub(super) fn sts_json(values: [&str; 4], skip: &str) -> String {
    let names = [
        "AccessKeyId",
        "SecretAccessKey",
        "SessionToken",
        "Expiration",
    ];
    let fields: Vec<String> = names
        .iter()
        .zip(values)
        .filter(|(name, _)| **name != skip)
        .map(|(name, value)| format!(r#""{name}":"{value}""#))
        .collect();
    format!(r#"{{"Credentials":{{{}}}}}"#, fields.join(","))
}

pub(super) fn response(expiration: &str) -> StsResponse {
    serde_json::from_str(&sts_json(["id", "secret", "token", expiration], "")).unwrap()
}

pub(super) fn now() -> DateTime<Utc> {
    "2026-01-01T00:00:00Z".parse().unwrap()
}

pub(super) fn returned_expiration_ttl() {
    let rows = [
        ("2026-01-01T01:00:00Z", 3540),
        ("2026-01-01T03:00:00+02:00", 3540),
        ("2026-01-01T00:02:00.700Z", 60),
        ("2026-01-01T00:01:00Z", 0),
        ("2026-01-01T00:00:30.500Z", -29),
        ("2025-12-31T23:00:00Z", -3660),
        ("2026-01-01T00:10:00", 540),
    ];
    for (expiration, expected) in rows {
        let (credentials, ttl) = extract_credentials_and_ttl(response(expiration), now()).unwrap();
        assert_eq!(ttl, expected, "{expiration}");
        assert_eq!(credentials.access_key_id.expose_secret(), "id");
        assert_eq!(credentials.secret_access_key.expose_secret(), "secret");
        assert_eq!(credentials.session_token.unwrap().expose_secret(), "token");
    }
    for missing in [
        "AccessKeyId",
        "SecretAccessKey",
        "SessionToken",
        "Expiration",
    ] {
        let body = sts_json(["a", "b", "c", "2026-01-01T01:00:00Z"], missing);
        let error = serde_json::from_str::<StsResponse>(&body).unwrap_err();
        assert!(error.to_string().contains(missing), "{missing}: {error}");
    }
    serde_json::from_str::<StsResponse>("{}").unwrap_err();
}
