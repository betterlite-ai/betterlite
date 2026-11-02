#[cfg(test)]
use rstest::rstest;
#[cfg(test)]
use secrecy::ExposeSecret;

#[cfg(test)]
use super::test_object;

#[cfg(test)]
struct RegistryStub {
    resolved: Option<Map<String, Value>>,
}

#[cfg(test)]
impl SandboxToolRegistry for RegistryStub {
    fn resolve(&self, _sandbox_tool_name: &str) -> Option<Map<String, Value>> {
        self.resolved.clone()
    }
}

#[cfg(test)]
#[test]
fn tool_resolution_returns_none_when_sandbox_tool_name_is_absent() {
    let registry = RegistryStub { resolved: None };
    assert!(resolve_sandbox_tool(None, &registry).is_none());
}

#[cfg(test)]
#[test]
fn tool_resolution_returns_none_when_the_registry_cannot_resolve_the_name() {
    let registry = RegistryStub { resolved: None };
    assert!(resolve_sandbox_tool(Some("missing-tool"), &registry).is_none());
}

#[cfg(test)]
#[test]
fn tool_resolution_returns_params_when_the_registry_resolves_the_name() {
    let registry = RegistryStub {
        resolved: Some(
            test_object([("sandbox_provider", Value::from("e2b"))])
                .as_object()
                .unwrap()
                .clone(),
        ),
    };
    let params = resolve_sandbox_tool(Some("my-tool"), &registry).unwrap();
    assert_eq!(params.sandbox_provider, "e2b");
}

#[cfg(test)]
#[rstest]
#[case::all_present(
    test_object([
        ("sandbox_provider", Value::from("e2b")),
        ("api_key", Value::from("secret")),
        ("api_base", Value::from("https://sandbox.example")),
    ]),
    SandboxToolParams {
        sandbox_provider: "e2b".to_owned(),
        api_key: Some(SecretString::from("secret")),
        api_base: Some("https://sandbox.example".to_owned()),
    }
)]
#[case::missing_provider(
    test_object([("api_key", Value::from("secret"))]),
    SandboxToolParams { sandbox_provider: String::new(), api_key: Some(SecretString::from("secret")), api_base: None }
)]
#[case::non_string_provider(
    test_object([("sandbox_provider", Value::from(5_i64))]),
    SandboxToolParams { sandbox_provider: String::new(), api_key: None, api_base: None }
)]
#[case::non_string_api_key(
    test_object([("sandbox_provider", Value::from("e2b")), ("api_key", Value::from(true))]),
    SandboxToolParams { sandbox_provider: "e2b".to_owned(), api_key: None, api_base: None }
)]
fn tool_extraction_coerces_resolved_fields_to_the_declared_types(
    #[case] resolved: Value,
    #[case] expected: SandboxToolParams,
) {
    let registry = RegistryStub {
        resolved: Some(resolved.as_object().unwrap().clone()),
    };
    let params = resolve_sandbox_tool(Some("tool"), &registry).unwrap();
    assert_eq!(params.sandbox_provider, expected.sandbox_provider);
    assert_eq!(
        params.api_key.as_ref().map(ExposeSecret::expose_secret),
        expected.api_key.as_ref().map(ExposeSecret::expose_secret)
    );
    assert_eq!(params.api_base, expected.api_base);
}
