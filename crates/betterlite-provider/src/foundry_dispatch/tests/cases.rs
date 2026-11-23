use rstest::rstest;

use super::{agent_cases, chat_cases, claude_cases, embedding_cases, http_cases};

#[derive(Clone, Copy)]
enum Case {
    Chat,
    Embedding,
    Anthropic,
    Agent,
    AgentsMissingBase,
    ClaudeMissingBase,
    ClaudeUrlRewrite,
    CommandR,
    Http,
    PostCall,
}

#[rstest]
#[case::foundry_chat_dispatch(Case::Chat)]
#[case::foundry_embedding_dispatch(Case::Embedding)]
#[case::foundry_anthropic_dispatch(Case::Anthropic)]
#[case::foundry_agent_dispatch(Case::Agent)]
#[case::agents_missing_base(Case::AgentsMissingBase)]
#[case::claude_missing_base(Case::ClaudeMissingBase)]
#[case::claude_url_rewrite(Case::ClaudeUrlRewrite)]
#[case::command_r_tool_content(Case::CommandR)]
#[case::foundry_provider_dispatch_http(Case::Http)]
#[case::foundry_post_call(Case::PostCall)]
fn foundry_dispatch_cases(#[case] case: Case) {
    match case {
        Case::Chat => chat_cases::chat_case(),
        Case::Embedding => embedding_cases::embedding_case(),
        Case::Anthropic => claude_cases::anthropic_case(),
        Case::Agent => agent_cases::agent_case(),
        Case::AgentsMissingBase => agent_cases::agents_missing_base(),
        Case::ClaudeMissingBase => claude_cases::claude_missing_base(),
        Case::ClaudeUrlRewrite => claude_cases::claude_url_rewrite(),
        Case::CommandR => chat_cases::command_r_case(),
        Case::Http => http_cases::http_case(),
        Case::PostCall => chat_cases::post_call_case(),
    }
}
