use super::encoding::encode_prompt_id;
use super::error::{LoadFailure, ManagerError};
use super::parse::{FrontmatterParser, parse_prompt_file};
use super::path::id_to_repo_path;
use super::template::GitLabPromptTemplate;
use crate::gitlab_client::GitLabClient;
use crate::gitlab_client::transport::GitLabTransport;

// A missing or empty file loads nothing, like Python's `if prompt_content:`.
pub(super) async fn fetch_template<T>(
    client: &GitLabClient<T>,
    prompts_path: &str,
    parser: FrontmatterParser,
    prompt_id: &str,
    ref_override: Option<&str>,
) -> Result<Option<GitLabPromptTemplate>, ManagerError>
where
    T: GitLabTransport,
{
    let file_path = id_to_repo_path(prompts_path, prompt_id);
    let load_error = |source: LoadFailure| ManagerError::Load {
        prompt_id: encode_prompt_id(prompt_id),
        source,
    };
    let fetched = client
        .get_file_content(&file_path, ref_override)
        .await
        .map_err(|source| load_error(LoadFailure::Fetch(source)))?;
    let Some(content) = fetched.filter(|text| !text.is_empty()) else {
        return Ok(None);
    };
    let template = parse_prompt_file(&content, prompt_id, parser)
        .map_err(|source| load_error(LoadFailure::Parse(source)))?;
    Ok(Some(template))
}

mod tests;
