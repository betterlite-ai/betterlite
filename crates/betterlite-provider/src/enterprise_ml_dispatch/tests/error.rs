use betterlite_openai::sagemaker_completion::error::SagemakerError;
use betterlite_openai::watsonx_auth::error::WatsonxError;

use crate::data_cloud_dispatch::handler::HandlerError;
use crate::enterprise_ml_dispatch::error::EnterpriseError;
use crate::sap_auth::error::SapAuthError;
use crate::sap_chat::error::SapChatError;
use crate::sap_embedding::error::SapEmbeddingError;
use crate::sap_stream::SapOrchestrationError;

#[test]
fn enterprise_error_carries_the_provider_status() {
    let handler = EnterpriseError::Handler(HandlerError::Upstream { status_code: 502 });
    assert_eq!(handler.status(), Some(502));
    let chat_credentials =
        EnterpriseError::SapChat(SapChatError::credentials(SapAuthError::NoCredentials));
    assert_eq!(chat_credentials.status(), Some(400));
    let embedding_credentials =
        EnterpriseError::SapEmbedding(SapEmbeddingError::Auth(SapAuthError::MissingCredentials));
    assert_eq!(embedding_credentials.status(), None);
    let orchestration = EnterpriseError::SapOrchestration(SapOrchestrationError {
        status_code: 418,
        message: String::from("teapot"),
    });
    assert_eq!(orchestration.status(), Some(418));
    let watsonx = EnterpriseError::Watsonx(Box::new(WatsonxError::Api {
        status_code: 401,
        message: String::from("no key"),
        headers: None,
    }));
    assert_eq!(watsonx.status(), Some(401));
    let sagemaker = EnterpriseError::Sagemaker(SagemakerError {
        status_code: 500,
        message: String::from("boom"),
        headers: None,
    });
    assert_eq!(sagemaker.status(), Some(500));
    let typed = EnterpriseError::FieldType { field: "apikey" };
    assert_eq!(typed.status(), None);
}

#[test]
fn watsonx_errors_convert_through_the_boxed_variant() {
    let converted = EnterpriseError::from(WatsonxError::IamApiKeyRequired);
    assert_eq!(converted.status(), None);
    assert_eq!(converted.to_string(), "API key is required");
    assert!(matches!(converted, EnterpriseError::Watsonx(_)));
}
