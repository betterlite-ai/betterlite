#![cfg(test)]

use betterlite_error::error::ErrorClass;

use super::super::credentials::VariationPolicies;
use super::super::entry::aimage_variation;
use super::support::{
    ERROR_BODY, FailingTopaz, Fixture, NoTopaz, RecordingTransport, SOURCE, SUCCESS_BODY, redactor,
    service, settings, variation_call,
};

#[tokio::test]
async fn openai_error_bodies_supply_the_class_message() {
    let fixture = Fixture::new();
    let context = fixture.context();
    let settings = settings(SOURCE, &[("OPENAI_API_KEY", "sk-test")]);
    let transport = RecordingTransport::new(400, ERROR_BODY);
    let service = service(
        &settings,
        &context,
        &transport,
        &NoTopaz,
        VariationPolicies::default(),
    );
    let error = aimage_variation(&service, &variation_call(), &redactor())
        .await
        .expect_err("the provider refuses the request");
    let ErrorClass::BadRequest(details) = &error.class else {
        panic!("a 400 body projects as a bad request");
    };
    // the SDK's body carries the provider's own message, unlike the "Error code: ..." text exception_type reads
    assert_eq!(details.message, "OpenAIException - nope");
}

#[tokio::test]
async fn openai_status_text_that_reads_as_request_too_large_wins() {
    let fixture = Fixture::new();
    let context = fixture.context();
    let settings = settings(SOURCE, &[("OPENAI_API_KEY", "sk-test")]);
    let body = br#"{"error":{"message":"Request too large for the model"}}"#;
    let transport = RecordingTransport::new(400, body);
    let service = service(
        &settings,
        &context,
        &transport,
        &NoTopaz,
        VariationPolicies::default(),
    );
    let error = aimage_variation(&service, &variation_call(), &redactor())
        .await
        .expect_err("the provider refuses the request");
    assert!(matches!(&error.class, ErrorClass::RateLimit(_)));
    assert_eq!(error.class.metadata().status_code, 429);
    assert_eq!(transport.take().len(), 1);
}

#[tokio::test]
async fn topaz_forbidden_projects_as_permission_denied() {
    let fixture = Fixture::new();
    let context = fixture.context();
    let settings = settings(SOURCE, &[("TOPAZ_API_KEY", "topaz-key")]);
    let transport = RecordingTransport::new(200, SUCCESS_BODY);
    let topaz = FailingTopaz::new(403, "no access");
    let service = service(
        &settings,
        &context,
        &transport,
        &topaz,
        VariationPolicies::default(),
    );
    let mut call = variation_call();
    call.custom_llm_provider = Some("topaz".to_owned());
    let error = aimage_variation(&service, &call, &redactor())
        .await
        .expect_err("the topaz executor is forbidden");
    assert_eq!(error.class.metadata().status_code, 403);
    assert!(matches!(&error.class, ErrorClass::PermissionDenied(_)));
    assert_eq!(error.message, "no access");
    assert_eq!(transport.take().len(), 0);
}

#[tokio::test]
async fn topaz_payment_required_projects_as_bad_request() {
    let fixture = Fixture::new();
    let context = fixture.context();
    let settings = settings(SOURCE, &[("TOPAZ_API_KEY", "topaz-key")]);
    let transport = RecordingTransport::new(200, SUCCESS_BODY);
    let topaz = FailingTopaz::new(402, "payment required");
    let service = service(
        &settings,
        &context,
        &transport,
        &topaz,
        VariationPolicies::default(),
    );
    let mut call = variation_call();
    call.custom_llm_provider = Some("topaz".to_owned());
    let error = aimage_variation(&service, &call, &redactor())
        .await
        .expect_err("the topaz executor refuses the payment");
    assert_eq!(error.class.metadata().status_code, 402);
    assert!(matches!(&error.class, ErrorClass::BadRequest(_)));
    assert_eq!(error.message, "payment required");
}

#[tokio::test]
async fn topaz_timeout_text_projects_as_a_timeout() {
    let fixture = Fixture::new();
    let context = fixture.context();
    let settings = settings(SOURCE, &[("TOPAZ_API_KEY", "topaz-key")]);
    let transport = RecordingTransport::new(200, SUCCESS_BODY);
    let topaz = FailingTopaz::new(500, "The read operation timed out");
    let service = service(
        &settings,
        &context,
        &transport,
        &topaz,
        VariationPolicies::default(),
    );
    let mut call = variation_call();
    call.custom_llm_provider = Some("topaz".to_owned());
    let error = aimage_variation(&service, &call, &redactor())
        .await
        .expect_err("the topaz executor timed out");
    assert_eq!(error.class.metadata().status_code, 408);
    assert!(matches!(&error.class, ErrorClass::Timeout(_)));
}
