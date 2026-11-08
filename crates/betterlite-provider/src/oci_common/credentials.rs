use secrecy::{ExposeSecret as _, SecretString};

pub const DEFAULT_OCI_REGION: &str = "us-ashburn-1";

#[derive(Debug, Clone, Default)]
pub struct OciExplicitParams {
    pub oci_region: Option<String>,
    pub oci_user: Option<String>,
    pub oci_fingerprint: Option<String>,
    pub oci_tenancy: Option<String>,
    pub oci_key: Option<SecretString>,
    pub oci_key_file: Option<String>,
    pub oci_compartment_id: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub struct OciEnvValues {
    pub oci_region: Option<String>,
    pub oci_user: Option<String>,
    pub oci_fingerprint: Option<String>,
    pub oci_tenancy: Option<String>,
    pub oci_key: Option<SecretString>,
    pub oci_key_file: Option<String>,
    pub oci_compartment_id: Option<String>,
}

#[derive(Debug, Clone)]
pub struct OciCredentials {
    pub oci_region: String,
    pub oci_user: Option<String>,
    pub oci_fingerprint: Option<String>,
    pub oci_tenancy: Option<String>,
    pub oci_key: Option<SecretString>,
    pub oci_key_file: Option<String>,
    pub oci_compartment_id: Option<String>,
}

#[must_use]
pub fn resolve_oci_credentials(explicit: &OciExplicitParams, env: &OciEnvValues) -> OciCredentials {
    OciCredentials {
        oci_region: non_empty(explicit.oci_region.as_deref(), env.oci_region.as_deref())
            .unwrap_or_else(|| DEFAULT_OCI_REGION.to_owned()),
        oci_user: non_empty(explicit.oci_user.as_deref(), env.oci_user.as_deref()),
        oci_fingerprint: non_empty(
            explicit.oci_fingerprint.as_deref(),
            env.oci_fingerprint.as_deref(),
        ),
        oci_tenancy: non_empty(explicit.oci_tenancy.as_deref(), env.oci_tenancy.as_deref()),
        oci_key: explicit
            .oci_key
            .as_ref()
            .filter(|value| !value.expose_secret().is_empty())
            .or_else(|| {
                env.oci_key
                    .as_ref()
                    .filter(|value| !value.expose_secret().is_empty())
            })
            .cloned(),
        oci_key_file: non_empty(
            explicit.oci_key_file.as_deref(),
            env.oci_key_file.as_deref(),
        ),
        oci_compartment_id: non_empty(
            explicit.oci_compartment_id.as_deref(),
            env.oci_compartment_id.as_deref(),
        ),
    }
}

fn non_empty(explicit: Option<&str>, env: Option<&str>) -> Option<String> {
    explicit
        .filter(|value| !value.is_empty())
        .or_else(|| env.filter(|value| !value.is_empty()))
        .map(str::to_owned)
}
