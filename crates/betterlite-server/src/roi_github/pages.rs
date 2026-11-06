use axum::http::HeaderMap;
use axum::http::Method;
use axum::http::header::LINK;
use futures_util::Stream;
use futures_util::stream::try_unfold;
use serde::de::DeserializeOwned;

use super::client::GitHub;
use super::error::SourceError;
use super::models::PAGINATION_ERROR;
use super::transport::Transport;

pub async fn fetch_page<M, T>(
    github: &GitHub<T>,
    path: &str,
    params: &[(String, String)],
    page: i64,
    error_message: &str,
) -> Result<(Vec<M>, bool), SourceError>
where
    M: DeserializeOwned,
    T: Transport,
{
    let mut page_params = params.to_vec();
    page_params.push(("per_page".to_owned(), "100".to_owned()));
    page_params.push(("page".to_owned(), page.to_string()));
    let response = github
        .request(Method::GET, path, &page_params, None)
        .await?;
    let items =
        serde_json::from_slice(&response.body).map_err(|error| SourceError::Pagination {
            message: error_message.to_owned(),
            source: error,
        })?;
    Ok((items, has_next_link(&response.headers)))
}

pub fn pages<'client, M, T>(
    github: &'client GitHub<T>,
    path: &'client str,
    params: &'client [(String, String)],
    limit: u32,
) -> impl Stream<Item = Result<Vec<M>, SourceError>> + Send + 'client
where
    M: DeserializeOwned + 'client,
    T: Transport,
{
    try_unfold(Some((1_u32, false)), move |state| async move {
        let Some((page, exhausted)) = state else {
            return Ok(None);
        };
        if exhausted || page > limit {
            return Err(SourceError::PaginationLimit);
        }
        let (items, has_next) =
            fetch_page::<M, T>(github, path, params, i64::from(page), PAGINATION_ERROR).await?;
        let next = (has_next).then(|| (page.saturating_add(1), page >= limit));
        Ok(Some((items, next)))
    })
}

pub(crate) fn has_next_link(headers: &HeaderMap) -> bool {
    // httpx joins repeated Link headers with ", " for Headers.get, so the whole value is searched
    let joined = headers
        .get_all(LINK)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .collect::<Vec<_>>()
        .join(", ");
    joined.contains("rel=\"next\"")
}
