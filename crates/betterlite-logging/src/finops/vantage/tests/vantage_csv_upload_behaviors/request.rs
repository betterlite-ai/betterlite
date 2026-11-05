use std::sync::Arc;

use rstest::rstest;
use secrecy::SecretString;

use crate::finops::vantage::destination::VantageDestination;
use crate::finops::vantage::settings::VantageSettings;
use crate::finops::vantage::tests::support::{FILENAME, FakeSender, Posted};

async fn post_with(base_url: Option<&str>) -> Posted {
    let sender = FakeSender::default();
    let posted = Arc::clone(&sender.posted);
    let settings = VantageSettings {
        api_key: Some(SecretString::from("key-1")),
        integration_token: Some(SecretString::from("token-1")),
        base_url: base_url.map(str::to_owned),
    };
    let destination = VantageDestination::new(sender, settings).unwrap();
    destination
        .deliver(b"ServiceName\nCompute\n", FILENAME)
        .await
        .unwrap();
    let captured = posted.lock().unwrap();
    assert_eq!(captured.len(), 1);
    captured[0].clone()
}

#[rstest]
#[case::default_base_url(None, "https://api.vantage.sh/v2/integrations/token-1/costs.csv")]
#[case::configured_base_url(
    Some("http://localhost:9000"),
    "http://localhost:9000/v2/integrations/token-1/costs.csv"
)]
#[case::trailing_slash_is_not_trimmed(
    Some("http://localhost:9000/"),
    "http://localhost:9000//v2/integrations/token-1/costs.csv"
)]
#[tokio::test]
async fn the_cost_import_url_comes_from_the_base_url_and_integration_token(
    #[case] base_url: Option<&str>,
    #[case] url: &str,
) {
    assert_eq!(post_with(base_url).await.url, url);
}

#[tokio::test]
async fn the_csv_goes_up_as_a_bearer_authorized_text_csv_file_part() {
    let posted = post_with(None).await;
    assert_eq!(posted.authorization, "Bearer key-1");
    assert_eq!(posted.field, "csv");
    assert_eq!(posted.filename, FILENAME);
    assert_eq!(posted.media_type, "text/csv");
    assert_eq!(posted.data, b"ServiceName\nCompute\n");
}
