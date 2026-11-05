use crate::gitlab_client::GitLabClient;
use crate::gitlab_client::config::GitLabConfig;
use crate::gitlab_client::error::GitLabClientError;
use crate::gitlab_client::transport::GitLabTransport;

// Python's `gitlab_client or GitLabClient(config)`.
pub enum ClientSource<T> {
    Provided(GitLabClient<T>),
    Transport(T),
}

// `prompts_path or folder or ""`, so an empty prompts_path defers to folder.
pub(super) fn scoped_prompts_path(config: &GitLabConfig) -> String {
    config
        .prompts_path
        .as_deref()
        .filter(|path| !path.is_empty())
        .or(config.folder.as_deref())
        .unwrap_or_default()
        .trim_matches('/')
        .to_owned()
}

pub(super) fn resolve_client<T>(
    config: &GitLabConfig,
    source: ClientSource<T>,
    ref_name: Option<&str>,
) -> Result<GitLabClient<T>, GitLabClientError>
where
    T: GitLabTransport,
{
    let mut client = match source {
        ClientSource::Provided(client) => client,
        ClientSource::Transport(transport) => GitLabClient::new(config, transport)?,
    };
    if let Some(requested) = ref_name.filter(|name| !name.is_empty()) {
        client.set_ref(requested)?;
    }
    Ok(client)
}

mod tests;
