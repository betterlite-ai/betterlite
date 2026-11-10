#![cfg(test)]

use rstest::rstest;

use crate::azure_hosted_identity::msal_reply::MsalKind;
use crate::azure_hosted_identity::protocol::HostedProtocol;

use super::fixture::{ArcFixture, Fixture, FixtureClock, NOW, only_request, response};
use super::scenarios::{
    APP_SERVICE_ENVIRONMENT, AZURE_ML_ENVIRONMENT, SERVICE_FABRIC_ENVIRONMENT, SF_TOKEN,
};

#[rstest]
#[case::app_client(MsalKind::AppService, &APP_SERVICE_ENVIRONMENT, &[("client_id", "cfg-client")], "client_id", "cfg-client")]
#[case::app_resource(MsalKind::AppService, &APP_SERVICE_ENVIRONMENT, &[("resource_id", "/subs/res")], "mi_res_id", "/subs/res")]
#[case::app_object(MsalKind::AppService, &APP_SERVICE_ENVIRONMENT, &[("object_id", "cfg-object")], "object_id", "cfg-object")]
#[case::app_resource_before_object(MsalKind::AppService, &APP_SERVICE_ENVIRONMENT, &[("object_id", "cfg-object"), ("resource_id", "/subs/res")], "mi_res_id", "/subs/res")]
#[case::app_client_before_resource(MsalKind::AppService, &APP_SERVICE_ENVIRONMENT, &[("resource_id", "/subs/res"), ("client_id", "cfg-client")], "client_id", "cfg-client")]
#[case::app_empty_client_skipped(MsalKind::AppService, &APP_SERVICE_ENVIRONMENT, &[("client_id", ""), ("object_id", "cfg-object")], "object_id", "cfg-object")]
#[case::ml_client(MsalKind::AzureMl, &AZURE_ML_ENVIRONMENT, &[("client_id", "cfg-client")], "clientid", "cfg-client")]
#[case::ml_resource(MsalKind::AzureMl, &AZURE_ML_ENVIRONMENT, &[("resource_id", "/subs/res")], "msi_res_id", "/subs/res")]
#[case::ml_object(MsalKind::AzureMl, &AZURE_ML_ENVIRONMENT, &[("object_id", "cfg-object")], "object_id", "cfg-object")]
#[case::ml_resource_before_object(MsalKind::AzureMl, &AZURE_ML_ENVIRONMENT, &[("object_id", "cfg-object"), ("resource_id", "/subs/res")], "msi_res_id", "/subs/res")]
fn identity_config_selects_user_assigned_identity(
    #[case] kind: MsalKind,
    #[case] environment: &[(&str, &str)],
    #[case] identity_config: &[(&str, &str)],
    #[case] name: &str,
    #[case] value: &str,
) {
    let (outcome, transport) = Fixture::new()
        .with_environment(environment)
        .with_identity_config(identity_config)
        .acquire(kind, vec![response(SF_TOKEN)], &FixtureClock::fixed(NOW));
    outcome.unwrap();
    let request = only_request(&transport);
    assert_eq!(request.query.len(), 3);
    assert_eq!(request.query_value(name), Some(value));
}

#[test]
fn identity_config_yields_to_client_id() {
    let (outcome, transport) = Fixture::new()
        .with_environment(&APP_SERVICE_ENVIRONMENT)
        .with_client_id("arg-client")
        .with_identity_config(&[("client_id", "cfg-client"), ("object_id", "cfg-object")])
        .acquire(
            MsalKind::AppService,
            vec![response(SF_TOKEN)],
            &FixtureClock::fixed(NOW),
        );
    outcome.unwrap();
    let request = only_request(&transport);
    assert_eq!(request.query.len(), 3);
    assert_eq!(request.query_value("client_id"), Some("arg-client"));
}

#[test]
fn identity_config_unrelated_keys_stay_system_assigned() {
    let (outcome, transport) = Fixture::new()
        .with_environment(&AZURE_ML_ENVIRONMENT)
        .with_identity_config(&[("tenant", "alpha")])
        .acquire(
            MsalKind::AzureMl,
            vec![response(SF_TOKEN)],
            &FixtureClock::fixed(NOW),
        );
    outcome.unwrap();
    assert_eq!(only_request(&transport).query.len(), 2);
}

pub(super) fn hosted_inner_selection() {
    presence_selection();
    arc_selection();
}

#[test]
fn presence_selection() {
    assert_eq!(
        ArcFixture::new()
            .with_environment(&SERVICE_FABRIC_ENVIRONMENT)
            .select(),
        HostedProtocol::ServiceFabric
    );
    assert_eq!(
        ArcFixture::new()
            .with_environment(&APP_SERVICE_ENVIRONMENT)
            .select(),
        HostedProtocol::AppService
    );
    assert_eq!(
        ArcFixture::new()
            .with_environment(&AZURE_ML_ENVIRONMENT)
            .select(),
        HostedProtocol::AzureMl
    );
    assert_eq!(
        ArcFixture::new()
            .with_environment(&[
                ("IDENTITY_ENDPOINT", "http://arc.test/token"),
                ("IMDS_ENDPOINT", "http://arc.test/imds"),
            ])
            .select(),
        HostedProtocol::Arc
    );
    assert_eq!(ArcFixture::new().select(), HostedProtocol::Vm);
}

#[rstest]
#[case::service_fabric(HostedProtocol::ServiceFabric, "service_fabric")]
#[case::app_service(HostedProtocol::AppService, "app_service")]
#[case::azure_ml(HostedProtocol::AzureMl, "azure_ml")]
#[case::arc(HostedProtocol::Arc, "arc")]
#[case::vm(HostedProtocol::Vm, "vm")]
fn protocol_names(#[case] protocol: HostedProtocol, #[case] name: &str) {
    assert_eq!(protocol.name(), name);
}

#[rstest]
#[case::msi_endpoint_alone(&[("MSI_ENDPOINT", "http://ml.test/msi")], false, HostedProtocol::Vm)]
#[case::msi_secret_alone(&[("MSI_SECRET", "secret")], false, HostedProtocol::Vm)]
#[case::identity_endpoint_alone(&[("IDENTITY_ENDPOINT", "http://arc.test/token")], false, HostedProtocol::Vm)]
#[case::imds_endpoint_alone_with_himds(&[("IMDS_ENDPOINT", "http://arc.test/imds")], true, HostedProtocol::Arc)]
fn partial_settings_fall_through_to_the_arc_check(
    #[case] environment: &[(&str, &str)],
    #[case] himds_installed: bool,
    #[case] expected: HostedProtocol,
) {
    let installed = ArcFixture::new().with_file("/opt/azcmagent/bin/himds", "");
    let host = if himds_installed {
        installed
    } else {
        ArcFixture::new()
    };
    assert_eq!(host.with_environment(environment).select(), expected);
}

#[test]
fn arc_selection() {
    assert_eq!(
        ArcFixture::new()
            .with_environment(&[
                ("IDENTITY_ENDPOINT", ""),
                ("IDENTITY_HEADER", ""),
                ("IDENTITY_SERVER_THUMBPRINT", ""),
            ])
            .select(),
        HostedProtocol::ServiceFabric
    );
    assert_eq!(
        ArcFixture::new()
            .with_environment(&[("IDENTITY_ENDPOINT", ""), ("IDENTITY_HEADER", "")])
            .select(),
        HostedProtocol::AppService
    );
    assert_eq!(
        ArcFixture::new()
            .with_environment(&[("MSI_ENDPOINT", ""), ("MSI_SECRET", "")])
            .select(),
        HostedProtocol::AzureMl
    );
    assert_eq!(
        ArcFixture::new()
            .with_environment(&[
                ("IDENTITY_ENDPOINT", ""),
                ("IMDS_ENDPOINT", "http://arc.test/imds"),
            ])
            .select(),
        HostedProtocol::Vm
    );
    assert_eq!(
        ArcFixture::new()
            .with_file("/opt/azcmagent/bin/himds", "")
            .select(),
        HostedProtocol::Arc
    );
}
