#![cfg(test)]

mod decode;
mod expiry;
mod items;
mod postgres;
mod store;
mod support;
mod types;

// ciphertexts written by LiteLLM 1.105.1 under LITELLM_SALT_KEY=b4713-salt
const ENCRYPTED_SECRET: &str =
    "BXspmMnYQwR-zqntcay3SaUe_tLavtlU2Eqtj0-aJUsw4AbmcRKk6I1ngAJfpqtDBgqpOA==";
const ENCRYPTED_TEXT: &str =
    "bwVtj4eUf8P0hulSl8_F9kgu5-OEzfJ7j4qDFox-C-xduSau8SmrsswJ1OP6QDe1JyPGn3PqPg==";
const ENCRYPTED_OAUTH: &str = "NYNVAzCgOZSXPasKJF981rgFj5kkXN6fAS2fybRn8ePKr1K7AFNVjYfDtkgA834E_ypvg-VTBvvgAo1RCNBGjBNHFfOEKORmetcd2VmmFblNAl4JsEF3ZxGBayKS5oMyQOjRMLYAk9KyNBqNa-WrqVgTgSL-aUKyRunxirMI8dujO4noECK5_O_ZoVx41c0JFqB7mxlpYeSMk5W2neWJhdjtcRnT1SKWVgVKRuFljHwwijDfx92bd9DQgj5GpD5Ul4xIT7oPzneX855RFcr9evXp8nPWaS3itDanNqnQihpevGsubaw=";

// the same values as plain URL-safe base64, the format rows written before encryption used
const LEGACY_SECRET: &str = "bGVnYWN5LXNlY3JldA==";
const LEGACY_BYOK: &str = "c2VjcmV0LXZhbHVl";
const LEGACY_OAUTH: &str = "eyJ0eXBlIjogIm9hdXRoMiIsICJhY2Nlc3NfdG9rZW4iOiAidG9rLTEyMyIsICJyZWZyZXNoX3Rva2VuIjogInJlZi00NTYiLCAiY29ubmVjdGVkX2F0IjogIjIwMjQtMDEtMDJUMDM6MDQ6MDUrMDA6MDAiLCAiZXhwaXJlc19hdCI6ICIyMDMwLTAxLTAyVDAzOjA0OjA1KzAwOjAwIiwgInNjb3BlcyI6IFsicmVhZCIsICJ3cml0ZSJdfQ==";
const NOT_JSON: &str = "bm90LWpzb257";
const UNDECRYPTABLE: &str = "_w==";

const OAUTH_PAYLOAD: &str = r#"{"type": "oauth2", "access_token": "tok-123", "refresh_token": "ref-456", "connected_at": "2024-01-02T03:04:05+00:00", "expires_at": "2030-01-02T03:04:05+00:00", "scopes": ["read", "write"]}"#;
