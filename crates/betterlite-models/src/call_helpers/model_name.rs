#[must_use]
pub fn reconstruct_model_name(
    model_name: &str,
    custom_llm_provider: Option<&str>,
    deployment_model_name: Option<&str>,
) -> String {
    if let Some(deployment) =
        deployment_model_name.filter(|name| !name.is_empty() && name.contains('/'))
    {
        return deployment.to_owned();
    }
    if custom_llm_provider == Some("bedrock") && !model_name.is_empty() && !model_name.contains('/')
    {
        return format!("bedrock/{model_name}");
    }
    model_name.to_owned()
}
