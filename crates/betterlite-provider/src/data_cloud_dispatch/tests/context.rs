use betterlite_prompts::arguments::json::{Json, Object};
use secrecy::ExposeSecret;

use crate::data_cloud_dispatch::context::{EnvironmentSource, oci_explicit_params};
use crate::data_cloud_dispatch::embed::databricks_environment;
use crate::oci_common::environment::DEFAULT_LITELLM_VERSION;

use super::lookup::secrets;
use super::support::environment;

#[test]
fn databricks_environment_reads_the_environment() {
    let secrets = secrets();
    let environment = environment(&[
        ("DATABRICKS_CLIENT_ID", "client-id"),
        ("DATABRICKS_CLIENT_SECRET", "client-secret"),
        ("DATABRICKS_API_BASE", "https://dbc.example.com"),
        ("LITELLM_USER_AGENT", "agent/1"),
        ("DATABRICKS_USER_AGENT", "agent/2"),
    ]);
    let source = EnvironmentSource {
        secrets: &secrets,
        environment: &environment,
    };
    let resolved = databricks_environment(&source).expect("the environment is read");
    assert_eq!(resolved.client_id.as_deref(), Some("client-id"));
    assert_eq!(
        resolved
            .client_secret
            .as_ref()
            .map(ExposeSecret::expose_secret),
        Some("client-secret")
    );
    assert_eq!(
        resolved.api_base.as_deref(),
        Some("https://dbc.example.com")
    );
    assert_eq!(resolved.user_agent_override.as_deref(), Some("agent/1"));
    assert_eq!(resolved.version, DEFAULT_LITELLM_VERSION);
}

#[test]
fn databricks_environment_falls_back_to_the_databricks_user_agent() {
    let secrets = secrets();
    let environment = environment(&[("DATABRICKS_USER_AGENT", "agent/2")]);
    let source = EnvironmentSource {
        secrets: &secrets,
        environment: &environment,
    };
    let resolved = databricks_environment(&source).expect("the environment is read");
    assert_eq!(resolved.user_agent_override.as_deref(), Some("agent/2"));
}

#[test]
fn an_empty_environment_value_counts_as_unset() {
    let secrets = secrets();
    let environment = environment(&[("DATABRICKS_API_BASE", ""), ("DATABRICKS_API_KEY", "")]);
    let source = EnvironmentSource {
        secrets: &secrets,
        environment: &environment,
    };
    assert_eq!(
        source
            .text("DATABRICKS_API_BASE")
            .expect("the setting is read"),
        None
    );
    assert!(
        source
            .secret("DATABRICKS_API_KEY")
            .expect("the setting is read")
            .is_none()
    );
}

#[test]
fn oci_explicit_params_read_the_optional_parameters() {
    let params = Object::from_iter([
        (
            String::from("oci_region"),
            Json::Text(String::from("us-ashburn-1")),
        ),
        (
            String::from("oci_compartment_id"),
            Json::Text(String::from("ocid1.compartment.oc1..aaaa")),
        ),
        (String::from("oci_key_file"), Json::Null),
    ]);
    let explicit = oci_explicit_params(&params).expect("the optional parameters are read");
    assert_eq!(explicit.oci_region.as_deref(), Some("us-ashburn-1"));
    assert_eq!(
        explicit.oci_compartment_id.as_deref(),
        Some("ocid1.compartment.oc1..aaaa")
    );
    assert!(explicit.oci_user.is_none());
    assert!(explicit.oci_key_file.is_none());
}
