use std::future::Future;
use std::time::Duration;

use betterlite_mcp::client::sessions::client::SessionClient;
use betterlite_mcp::client::sessions::error::SessionError;
use betterlite_mcp::client::sessions::run::run_with_session;
use betterlite_mcp::client::sessions::runtime::McpRuntime;

use super::failure::{list_failure, unsupported_failure};
use super::operations::server_name;
use super::pagination::{PageFailure, list_optional_pages};
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

impl<Session: ResourceSession> ListKind<Session> for Prompts {
    type Item = Prompt;
    type Page = PromptPage;

    const OPERATION: &'static str = "list_prompts";

    fn is_supported(capabilities: &ServerCapabilities) -> bool {
        capabilities.prompts.is_some()
    }

    fn fetch(
        session: &mut Session,
        cursor: Option<String>,
    ) -> impl Future<Output = Result<Self::Page, Session::Error>> + Send {
        session.list_prompts(cursor)
    }

    fn items(page: &Self::Page) -> &[Self::Item] {
        &page.prompts
    }
}

impl<Session: ResourceSession> ListKind<Session> for Resources {
    type Item = Resource;
    type Page = ResourcePage;

    const OPERATION: &'static str = "list_resources";

    fn is_supported(capabilities: &ServerCapabilities) -> bool {
        capabilities.resources.is_some()
    }

    fn fetch(
        session: &mut Session,
        cursor: Option<String>,
    ) -> impl Future<Output = Result<Self::Page, Session::Error>> + Send {
        session.list_resources(cursor)
    }

    fn items(page: &Self::Page) -> &[Self::Item] {
        &page.resources
    }
}

impl<Session: ResourceSession> ListKind<Session> for Templates {
    type Item = ResourceTemplate;
    type Page = ResourceTemplatePage;

    const OPERATION: &'static str = "list_resource_templates";

    fn is_supported(capabilities: &ServerCapabilities) -> bool {
        capabilities.resources.is_some()
    }

    fn fetch(
        session: &mut Session,
        cursor: Option<String>,
    ) -> impl Future<Output = Result<Self::Page, Session::Error>> + Send {
        session.list_resource_templates(cursor)
    }

    fn items(page: &Self::Page) -> &[Self::Item] {
        &page.resource_templates
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
    let deadline = client.auth.timeout.max(settings.listing_timeout);
    let server = server_name(client);
    let operation = async |session: &mut RuntimeSession<Runtime>| {
        optional_list::<_, Kind>(session, deadline, &server).await
    };
    match run_with_session(client, operation, false).await {
        Ok(items) => Ok(items),
        Err(error) => list_failure(error, raise_on_error, Kind::OPERATION, &server),
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
        Err(PageFailure::Session(error)) => unsupported_failure(*error, Kind::OPERATION, server),
        Err(PageFailure::Unavailable(failure)) => Err(SessionError::Protocol(failure)),
    }
}
