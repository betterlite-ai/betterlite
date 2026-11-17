use betterlite_config::sections::azure_managed_identity::Settings;
use betterlite_redact::secret::Redactor;
use secrecy::{ExposeSecret as _, SecretString};

use crate::azure_arc_identity::error::ArcIdentityError;
use crate::azure_hosted_identity::error::HostedIdentityError;
use crate::azure_hosted_identity::protocol::HostedProtocol;

use super::super::environment::ManagedIdentityEnvironment;
use super::super::error::ManagedIdentityError;
use super::super::selection::{ManagedIdentityKind, select};
use super::cases::{SCOPE, workload_environment};
use super::fixture::{APP_SERVICE, Fixture, NOW, Reply};

const THUMBPRINT: &str = "00112233445566778899aabbccddeeff00112233";

pub(super) fn hosted_selection_order(redactor: &Redactor) {
    hosted_variants(redactor);
    workload_and_unconfigured(redactor);
    presence_over_truthiness(redactor);
    arc_file_probe(redactor);
    arc_token(redactor);
    hosted_tokens(redactor);
    hosted_and_arc_client_ids(redactor);
    kind_names();
    empty_partner_values(redactor);
    empty_workload_values(redactor);
    settings_environment();
}

fn selection_of(fixture: &Fixture<'_>) -> (ManagedIdentityKind, Option<&'static str>) {
    let credential = fixture.credential(None);
    let selection = credential.select().unwrap();
    let inner = credential
        .inner_protocol(&selection)
        .map(HostedProtocol::name);
    (selection.kind, inner)
}

fn hosted_variants(redactor: &Redactor) {
    let variants = [
        (
            vec![
                ("IDENTITY_ENDPOINT", "http://sf.test/token"),
                ("IDENTITY_HEADER", "header-secret"),
                ("IDENTITY_SERVER_THUMBPRINT", THUMBPRINT),
            ],
            (ManagedIdentityKind::ServiceFabric, Some("service_fabric")),
        ),
        (
            vec![
                ("IDENTITY_ENDPOINT", APP_SERVICE),
                ("IDENTITY_HEADER", "header-secret"),
            ],
            (ManagedIdentityKind::AppService, Some("app_service")),
        ),
        (
            vec![
                ("IDENTITY_ENDPOINT", "http://arc.test/token"),
                ("IMDS_ENDPOINT", "http://arc.test/imds"),
            ],
            (ManagedIdentityKind::Arc, Some("arc")),
        ),
        (
            vec![
                ("MSI_ENDPOINT", "http://ml.test/token"),
                ("MSI_SECRET", "ml-secret"),
            ],
            (ManagedIdentityKind::AzureMl, Some("azure_ml")),
        ),
    ];
    for (variables, expected) in variants {
        let fixture = Fixture::new(&variables, redactor);
        assert_eq!(selection_of(&fixture), expected, "{variables:?}");
    }
}

fn hosted_tokens(redactor: &Redactor) {
    let azure_ml = Fixture::new(
        &[
            ("MSI_ENDPOINT", "http://ml.test/token"),
            ("MSI_SECRET", "ml-secret"),
        ],
        redactor,
    );
    azure_ml.hosted.push(Reply::ok(
        r#"{"access_token":"ml-token","expires_on":1700003600}"#,
    ));
    let ml_token = azure_ml.acquire(&[SCOPE]).unwrap();
    assert_eq!(ml_token.token.expose_secret(), "ml-token");
    assert_eq!(azure_ml.hosted.recorded()[0].url, "http://ml.test/token");
    let service_fabric = Fixture::new(
        &[
            ("IDENTITY_ENDPOINT", "http://sf.test/token"),
            ("IDENTITY_HEADER", "header-secret"),
            ("IDENTITY_SERVER_THUMBPRINT", THUMBPRINT),
        ],
        redactor,
    );
    service_fabric.hosted.push(Reply::ok(
        r#"{"access_token":"sf-token","expires_on":1700003600,"token_type":"Bearer"}"#,
    ));
    let fabric_token = service_fabric.acquire(&[SCOPE]).unwrap();
    assert_eq!(fabric_token.token.expose_secret(), "sf-token");
    assert_eq!(
        service_fabric.hosted.recorded()[0].url,
        "http://sf.test/token"
    );
    let cloud_shell = Fixture::new(&[("MSI_ENDPOINT", "http://shell.test/token")], redactor);
    cloud_shell.hosted.push(Reply::ok(
        r#"{"access_token":"shell-token","expires_in":3600,"resource":"https://cognitiveservices.azure.com","token_type":"Bearer"}"#,
    ));
    let shell_token = cloud_shell.acquire(&[SCOPE]).unwrap();
    assert_eq!(shell_token.token.expose_secret(), "shell-token");
    assert_eq!(
        cloud_shell.hosted.recorded()[0].url,
        "http://shell.test/token"
    );
}

fn rejected_client_id(fixture: &Fixture<'_>) -> ManagedIdentityError {
    fixture
        .credential(Some("user-client"))
        .token(&fixture.io(), &[SCOPE])
        .unwrap_err()
}

// the client id reaches the hosted protocols: Service Fabric and Cloud Shell reject it, and Arc takes a user-assigned identity only through a non-empty one, which the SDK rejects
fn hosted_and_arc_client_ids(redactor: &Redactor) {
    let service_fabric = Fixture::new(
        &[
            ("IDENTITY_ENDPOINT", "http://sf.test/token"),
            ("IDENTITY_HEADER", "header-secret"),
            ("IDENTITY_SERVER_THUMBPRINT", THUMBPRINT),
        ],
        redactor,
    );
    assert!(matches!(
        rejected_client_id(&service_fabric),
        ManagedIdentityError::Hosted(HostedIdentityError::ServiceFabricIdentityConfig)
    ));
    let cloud_shell = Fixture::new(&[("MSI_ENDPOINT", "http://shell.test/token")], redactor);
    assert!(matches!(
        rejected_client_id(&cloud_shell),
        ManagedIdentityError::Hosted(HostedIdentityError::CloudShellClientId)
    ));
    assert!(cloud_shell.hosted.recorded().is_empty());
    assert!(matches!(
        rejected_client_id(&arc_fixture(redactor)),
        ManagedIdentityError::Arc(ArcIdentityError::UserAssignedIdentity)
    ));
    let system = arc_fixture(redactor);
    let accepted = system
        .credential(Some(""))
        .token(&system.io(), &[SCOPE])
        .unwrap();
    assert_eq!(accepted.token.expose_secret(), "arc-token");
}

fn settings_with(entries: &[(&str, &str)]) -> Settings {
    let text = |key: &str| {
        entries
            .iter()
            .find(|(name, _)| *name == key)
            .map(|(_, value)| (*value).to_owned())
    };
    Settings {
        identity_endpoint: text("identity_endpoint"),
        identity_header: text("identity_header").map(SecretString::from),
        identity_server_thumbprint: text("identity_server_thumbprint"),
        imds_endpoint: text("imds_endpoint"),
        msi_endpoint: text("msi_endpoint"),
        msi_secret: text("msi_secret").map(SecretString::from),
        pod_identity_authority_host: text("pod_identity_authority_host"),
    }
}

// the configured settings select the same protocols as the process environment
fn settings_environment() {
    let cases = [
        (
            settings_with(&[
                ("identity_endpoint", "http://sf.test/token"),
                ("identity_header", "header-secret"),
                ("identity_server_thumbprint", THUMBPRINT),
                ("pod_identity_authority_host", "http://pod.test"),
            ]),
            ManagedIdentityKind::ServiceFabric,
        ),
        (
            settings_with(&[
                ("identity_endpoint", "http://arc.test/token"),
                ("imds_endpoint", "http://arc.test/imds"),
            ]),
            ManagedIdentityKind::Arc,
        ),
        (
            settings_with(&[
                ("msi_endpoint", "http://ml.test/token"),
                ("msi_secret", "ml-secret"),
            ]),
            ManagedIdentityKind::AzureMl,
        ),
    ];
    for (settings, kind) in cases {
        let environment = ManagedIdentityEnvironment::from_settings(&settings);
        assert_eq!(select(&environment, None, false).unwrap().kind, kind);
        assert_eq!(
            environment.pod_identity_authority_host,
            settings.pod_identity_authority_host
        );
    }
}

fn kind_names() {
    let names = [
        (ManagedIdentityKind::ServiceFabric, "service_fabric"),
        (ManagedIdentityKind::AppService, "app_service"),
        (ManagedIdentityKind::Arc, "arc"),
        (ManagedIdentityKind::AzureMl, "azure_ml"),
        (ManagedIdentityKind::CloudShell, "cloud_shell"),
        (ManagedIdentityKind::Workload, "workload"),
        (ManagedIdentityKind::Imds, "imds"),
        (ManagedIdentityKind::Unconfigured, "none"),
    ];
    for (kind, name) in names {
        assert_eq!(kind.name(), name);
    }
}

// the outer selector treats an empty partner value as absent
fn empty_partner_values(redactor: &Redactor) {
    let shell = Fixture::new(
        &[
            ("MSI_ENDPOINT", "http://shell.test/token"),
            ("MSI_SECRET", ""),
        ],
        redactor,
    );
    assert_eq!(selection_of(&shell).0, ManagedIdentityKind::CloudShell);
    let arc = Fixture::new(
        &[
            ("IDENTITY_ENDPOINT", APP_SERVICE),
            ("IDENTITY_HEADER", ""),
            ("IMDS_ENDPOINT", "http://arc.test/imds"),
        ],
        redactor,
    );
    assert_eq!(selection_of(&arc).0, ManagedIdentityKind::Arc);
    let app_service = Fixture::new(
        &[
            ("IDENTITY_ENDPOINT", APP_SERVICE),
            ("IDENTITY_HEADER", "header-secret"),
            ("IDENTITY_SERVER_THUMBPRINT", ""),
        ],
        redactor,
    );
    assert_eq!(
        selection_of(&app_service).0,
        ManagedIdentityKind::AppService
    );
}

fn empty_workload_values(redactor: &Redactor) {
    for empty in [
        "AZURE_AUTHORITY_HOST",
        "AZURE_TENANT_ID",
        "AZURE_FEDERATED_TOKEN_FILE",
    ] {
        let mut variables = workload_environment();
        for pair in &mut variables {
            if pair.0 == empty {
                pair.1 = "";
            }
        }
        let fixture = Fixture::new(&variables, redactor);
        assert_eq!(selection_of(&fixture).0, ManagedIdentityKind::Imds);
    }
}

fn workload_and_unconfigured(redactor: &Redactor) {
    let cloud_shell = Fixture::new(&[("MSI_ENDPOINT", "http://ml.test/token")], redactor);
    assert_eq!(
        selection_of(&cloud_shell),
        (ManagedIdentityKind::CloudShell, None)
    );
    let workload = Fixture::new(
        &[
            ("AZURE_AUTHORITY_HOST", "https://login.contoso.test"),
            ("AZURE_TENANT_ID", "fixture-tenant"),
            ("AZURE_FEDERATED_TOKEN_FILE", "/var/run/token"),
            ("AZURE_CLIENT_ID", "env-client"),
        ],
        redactor,
    );
    assert_eq!(
        selection_of(&workload),
        (ManagedIdentityKind::Workload, None)
    );
    let imds = Fixture::new(&[], redactor);
    assert_eq!(selection_of(&imds), (ManagedIdentityKind::Imds, Some("vm")));
    assert!(imds.imds.recorded().is_empty());
    let lone = Fixture::new(&[("IDENTITY_ENDPOINT", APP_SERVICE)], redactor);
    assert_eq!(
        selection_of(&lone),
        (ManagedIdentityKind::Unconfigured, None)
    );
}

fn presence_over_truthiness(redactor: &Redactor) {
    let empty_all = Fixture::new(
        &[
            ("IDENTITY_ENDPOINT", ""),
            ("IDENTITY_HEADER", ""),
            ("IDENTITY_SERVER_THUMBPRINT", ""),
        ],
        redactor,
    );
    assert_eq!(
        selection_of(&empty_all),
        (ManagedIdentityKind::Imds, Some("service_fabric"))
    );
    let empty_pair = Fixture::new(
        &[("IDENTITY_ENDPOINT", ""), ("IDENTITY_HEADER", "")],
        redactor,
    );
    assert_eq!(
        selection_of(&empty_pair),
        (ManagedIdentityKind::Imds, Some("app_service"))
    );
    let empty_ml = Fixture::new(&[("MSI_ENDPOINT", ""), ("MSI_SECRET", "")], redactor);
    assert_eq!(
        selection_of(&empty_ml),
        (ManagedIdentityKind::Imds, Some("azure_ml"))
    );
    let empty_arc = Fixture::new(
        &[
            ("IDENTITY_ENDPOINT", ""),
            ("IMDS_ENDPOINT", "http://arc.test/imds"),
        ],
        redactor,
    );
    assert_eq!(
        selection_of(&empty_arc),
        (ManagedIdentityKind::Imds, Some("vm"))
    );
}

fn arc_file_probe(redactor: &Redactor) {
    let fixture = Fixture::new(&[], redactor).with_file("/opt/azcmagent/bin/himds", "");
    assert_eq!(
        selection_of(&fixture),
        (ManagedIdentityKind::Imds, Some("arc"))
    );
}

fn arc_fixture(redactor: &Redactor) -> Fixture<'_> {
    let fixture = Fixture::new(
        &[
            ("IDENTITY_ENDPOINT", "http://arc.test/token"),
            ("IMDS_ENDPOINT", "http://arc.test/imds"),
        ],
        redactor,
    )
    .with_file("/var/opt/azcmagent/tokens/secret.key", "arc-secret");
    fixture.arc.push(Reply::Response {
        status: 401,
        headers: vec![("WWW-Authenticate", "Basic realm=secret.key")],
        body: String::new(),
    });
    fixture.arc.push(Reply::ok(
        r#"{"access_token":"arc-token","expires_in":3600}"#,
    ));
    fixture
}

fn arc_token(redactor: &Redactor) {
    let fixture = arc_fixture(redactor);
    let token = fixture.acquire(&[SCOPE]).unwrap();
    assert_eq!(token.token.expose_secret(), "arc-token");
    assert_eq!(token.expires_on, Some(NOW + 3600));
    assert_eq!(token.refresh_on, None);
    assert!(fixture.imds.recorded().is_empty());
}
