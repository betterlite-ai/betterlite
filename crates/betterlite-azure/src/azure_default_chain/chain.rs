use super::error::{EXHAUSTED, TROUBLESHOOT_URL};
use super::grant::{ChainToken, Grant, LinkError};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChainName {
    DefaultAzureCredential,
    ChainedTokenCredential,
}

impl ChainName {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::DefaultAzureCredential => "DefaultAzureCredential",
            Self::ChainedTokenCredential => "ChainedTokenCredential",
        }
    }
}

pub enum LinkInner<G> {
    Ready(G),
    Unavailable(String),
}

pub enum Link<G> {
    Environment(LinkInner<G>),
    Workload(LinkInner<G>),
    ManagedIdentity(G),
}

impl<G: Grant> Link<G> {
    #[must_use]
    pub const fn name(&self) -> &'static str {
        match self {
            Self::Environment(_) => "EnvironmentCredential",
            Self::Workload(_) => "WorkloadIdentityCredential",
            Self::ManagedIdentity(_) => "ManagedIdentityCredential",
        }
    }

    fn token(&self) -> Result<ChainToken, LinkError> {
        match self {
            Self::Environment(inner) | Self::Workload(inner) => match inner {
                LinkInner::Ready(grant) => grant.token(),
                LinkInner::Unavailable(message) => Err(LinkError::Unavailable(message.clone())),
            },
            Self::ManagedIdentity(grant) => grant.token(),
        }
    }
}

pub struct Chain<G> {
    pub name: ChainName,
    pub links: Vec<Link<G>>,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct ChainFailure {
    pub chain: &'static str,
    pub message: String,
    pub attempted: Vec<(String, String)>,
}

impl<G: Grant> Chain<G> {
    pub fn token(&self) -> Result<ChainToken, ChainFailure> {
        let mut attempted = Vec::new();
        for link in &self.links {
            match link.token() {
                Ok(token) => {
                    tracing::info!(
                        "{} acquired a token from {}",
                        self.name.as_str(),
                        link.name()
                    );
                    return Ok(token);
                }
                Err(LinkError::Unavailable(message)) => {
                    attempted.push((link.name().to_owned(), message));
                }
                Err(LinkError::Authentication(message)) => {
                    attempted.push((link.name().to_owned(), message));
                    break;
                }
            }
        }
        let message = exhaustion_message(self.name, &attempted);
        tracing::warn!("{message}");
        Err(ChainFailure {
            chain: self.name.as_str(),
            message,
            attempted,
        })
    }
}

fn exhaustion_message(name: ChainName, attempted: &[(String, String)]) -> String {
    let lines: Vec<String> = attempted
        .iter()
        .map(|(credential, message)| {
            if message.is_empty() {
                credential.clone()
            } else {
                format!("{credential}: {message}")
            }
        })
        .collect();
    format!(
        "{} {EXHAUSTED}\nAttempted credentials:\n\t{}\nTo mitigate this issue, please refer to the troubleshooting guidelines here at {TROUBLESHOOT_URL}.",
        name.as_str(),
        lines.join("\n\t")
    )
}
