use betterlite_openai::chat::deployment::Provider as DeploymentProvider;

use crate::aiohttp_openai_dispatch::AiohttpOpenAi;
use crate::prepared::deployment::Deployment;
use crate::provider::Provider;

use super::listed;

#[test]
fn provider_framework_tests_behavior() {
    let openai = Provider::from_deployment(&listed("openai/up", "http://127.0.0.1:9/v1"))
        .expect("openai resolves");
    assert_eq!(openai.name(), "openai");
    let aiohttp = Provider::from_deployment(&listed("aiohttp_openai/up", "http://127.0.0.1:9"))
        .expect("aiohttp resolves");
    assert!(matches!(aiohttp, Provider::AiohttpOpenAi(AiohttpOpenAi)));
    let azure = Provider::from_deployment(&listed("azure/up", "http://127.0.0.1:9"))
        .expect("azure resolves to its stub");
    assert_eq!(azure.name(), "azure");

    let prefixed = listed("aiohttp_openai/org/up", "http://127.0.0.1:9");
    assert_eq!(Deployment::from_chat(&prefixed).model, "org/up");
    let mut organisation = listed("azure/up", "http://127.0.0.1:9");
    organisation.provider = DeploymentProvider::Unsupported {
        model: String::from("meta/llama"),
    };
    assert_eq!(Deployment::from_chat(&organisation).model, "meta/llama");
}
