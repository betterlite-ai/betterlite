use crate::provider_model_sets::{ProviderModelSet, ProviderModelSets};

fn set_for(provider: &str) -> Option<ProviderModelSet> {
    Some(match provider {
        "openai" => ProviderModelSet::OpenAiChatCompletion,
        "text-completion-openai" => ProviderModelSet::OpenAiTextCompletion,
        "cohere" => ProviderModelSet::Cohere,
        "anthropic" => ProviderModelSet::Anthropic,
        "openrouter" => ProviderModelSet::Openrouter,
        "vercel_ai_gateway" => ProviderModelSet::VercelAiGateway,
        "edenai" => ProviderModelSet::Edenai,
        "vertex_ai-text-models" => ProviderModelSet::VertexText,
        "vertex_ai-code-text-models" => ProviderModelSet::VertexCodeText,
        "vertex_ai-chat-models" => ProviderModelSet::VertexChat,
        "vertex_ai-code-chat-models" => ProviderModelSet::VertexCodeChat,
        "ai21" => ProviderModelSet::Ai21,
        "nlp_cloud" => ProviderModelSet::NlpCloud,
        "aleph_alpha" => ProviderModelSet::AlephAlpha,
        "bedrock" => ProviderModelSet::Bedrock,
        "novita" => ProviderModelSet::Novita,
        _ => return None,
    })
}

pub(super) fn add_registered_model(sets: &mut ProviderModelSets, provider: &str, key: &str) {
    let Some(set) = set_for(provider) else {
        return;
    };
    // openrouter stores the part after the first "/", every other provider the whole key
    let name = if set == ProviderModelSet::Openrouter {
        key.split_once('/').map_or(key, |(_, rest)| rest)
    } else {
        key
    };
    sets.sets.entry(set).or_default().insert(name.to_owned());
}
