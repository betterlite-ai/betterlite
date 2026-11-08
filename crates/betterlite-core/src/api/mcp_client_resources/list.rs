use std::future::Future;
use std::time::Duration;

use betterlite_mcp::client::sessions::client::SessionClient;
use betterlite_mcp::client::sessions::error::SessionError;
use betterlite_mcp::client::sessions::run::run_with_session;
use betterlite_mcp::client::sessions::runtime::McpRuntime;
use tracing::debug;

use super::failure::{FailureMode, finish_failure};
use super::operations::server_name;
use super::pagination::{METHOD_NOT_FOUND, PageFailure, list_optional_pages};
use super::session::{ResourceSession, RuntimeSession, session_error};
use super::settings::ResourceSettings;
use super::types::{
    PageCursor, Prompt, PromptPage, Resource, ResourcePage, ResourceTemplate, ResourceTemplatePage,
    ServerCapabilities,
};

pub struct Prompts;
pub struct Resources;
pub struct Templates;

pub trait ListKind<Session>
where
    Session: ResourceSession,
{
    type Item: Clone;
    type Page: PageCursor;

    const OPERATION: &'static str;

    fn is_supported(capabilities: &ServerCapabilities) -> bool;

    fn fetch(
        session: &mut Session,
        cursor: Option<String>,
    ) -> impl Future<Output = Result<Self::Page, Session::Error>> + Send;

    fn items(page: &Self::Page) -> &[Self::Item];
}

macro_rules! list_kind {
    ($kind:ident, $item:ty, $page:ty, $operation:literal, $capability:ident, $fetch:ident, $items:ident) => {
        impl<Session: ResourceSession> ListKind<Session> for $kind {
            type Item = $item;
            type Page = $page;

            const OPERATION: &'static str = $operation;

            fn is_supported(capabilities: &ServerCapabilities) -> bool {
                capabilities.$capability.is_some()
            }

            fn fetch(
                session: &mut Session,
                cursor: Option<String>,
            ) -> impl Future<Output = Result<Self::Page, Session::Error>> + Send {
                session.$fetch(cursor)
            }

            fn items(page: &Self::Page) -> &[Self::Item] {
                &page.$items
            }
        }
    };
}

list_kind!(
    Prompts,
    Prompt,
    PromptPage,
    "list_prompts",
    prompts,
    list_prompts,
    prompts
);
list_kind!(
    Resources,
    Resource,
    ResourcePage,
    "list_resources",
    resources,
    list_resources,
    resources
);
list_kind!(
    Templates,
    ResourceTemplate,
    ResourceTemplatePage,
    "list_resource_templates",
    resources,
    list_resource_templates,
    resource_templates
);

pub async fn list_from_session<Session, Kind>(
    session: &mut Session,
    client_timeout: Duration,
    listing_timeout: Duration,
    raise_on_error: bool,
    server: &str,
) -> Result<Vec<Kind::Item>, SessionError<Session::Error>>
where
    Session: ResourceSession,
    Kind: ListKind<Session>,
{
    let deadline = client_timeout.max(listing_timeout);
    match optional_list::<Session, Kind>(session, deadline, server).await {
        Ok(items) => Ok(items),
        Err(error) => finish_failure(
            error,
            FailureMode::List { raise_on_error },
            Kind::OPERATION,
            server,
        ),
    }
}

pub(crate) async fn run_optional_list<Runtime, Kind>(
    client: &mut SessionClient<Runtime>,
    settings: &ResourceSettings,
    raise_on_error: bool,
) -> Result<Vec<Kind::Item>, SessionError<Runtime::Error>>
where
    Runtime: McpRuntime,
    RuntimeSession<Runtime>: ResourceSession<Error = Runtime::Error>,
    Kind: ListKind<RuntimeSession<Runtime>>,
{
    let server = server_name(client);
    let client_timeout = client.auth.timeout;
    let listing_timeout = settings.listing_timeout;
    let operation = async |session: &mut RuntimeSession<Runtime>| {
        Ok(list_from_session::<_, Kind>(
            session,
            client_timeout,
            listing_timeout,
            raise_on_error,
            &server,
        )
        .await)
    };
    match run_with_session(client, operation, false).await {
        Ok(outcome) => outcome,
        Err(error) => finish_failure(
            error,
            FailureMode::List { raise_on_error },
            Kind::OPERATION,
            &server,
        ),
    }
}

async fn optional_list<Session, Kind>(
    session: &mut Session,
    deadline: Duration,
    server: &str,
) -> Result<Vec<Kind::Item>, SessionError<Session::Error>>
where
    Session: ResourceSession,
    Kind: ListKind<Session>,
{
    if session
        .server_capabilities()
        .is_some_and(|capabilities| !Kind::is_supported(&capabilities))
    {
        return Ok(Vec::new());
    }
    let list_session = session;
    let pages = list_optional_pages(
        deadline,
        async |cursor| match Kind::fetch(&mut *list_session, cursor).await {
            Ok(page) => Ok(page),
            Err(error) => Err(session_error(&*list_session, error)),
        },
        Kind::items,
    )
    .await;
    match pages {
        Ok(items) => Ok(items),
        Err(PageFailure::Session(error)) => {
            if let SessionError::Protocol(failure) = &*error
                && failure.code == METHOD_NOT_FOUND
            {
                debug!(
                    "MCP client {} is unsupported by {server}: {failure}",
                    Kind::OPERATION
                );
                return Ok(Vec::new());
            }
            Err(*error)
        }
        Err(PageFailure::Unavailable(failure)) => Err(SessionError::Protocol(failure)),
    }
}
