use axum::http::Method;

use super::adapters::{RepositoryValue, repository_values};
use super::client::GitHub;
use super::error::SourceError;
use super::models::{REPOSITORY_PAGE_ERROR, REPOSITORY_SEARCH_PAGES, RepositoryItem};
use super::pages::fetch_page;
use super::transport::Transport;

struct RepositoryPage {
    repositories: Vec<RepositoryItem>,
    has_more: bool,
}

pub async fn repositories<T>(
    github: &GitHub<T>,
    query: &str,
    page: i64,
) -> Result<(Vec<RepositoryValue>, bool), SourceError>
where
    T: Transport,
{
    let params = vec![
        ("sort".to_owned(), "updated".to_owned()),
        ("direction".to_owned(), "desc".to_owned()),
        (
            "affiliation".to_owned(),
            "owner,collaborator,organization_member".to_owned(),
        ),
    ];
    let found = if query.is_empty() {
        repository_page(github, &params, page).await?
    } else {
        search_repositories(github, query, page, &params).await?
    };
    Ok((repository_values(&found.repositories), found.has_more))
}

async fn repository_page<T>(
    github: &GitHub<T>,
    params: &[(String, String)],
    page: i64,
) -> Result<RepositoryPage, SourceError>
where
    T: Transport,
{
    let (repositories, has_more) =
        fetch_page::<RepositoryItem, T>(github, "user/repos", params, page, REPOSITORY_PAGE_ERROR)
            .await?;
    Ok(RepositoryPage {
        repositories,
        has_more,
    })
}

async fn search_repositories<T>(
    github: &GitHub<T>,
    query: &str,
    page: i64,
    params: &[(String, String)],
) -> Result<RepositoryPage, SourceError>
where
    T: Transport,
{
    let normalized_query = caseless::default_case_fold_str(query);
    let first_page = page
        .saturating_sub(1)
        .saturating_mul(REPOSITORY_SEARCH_PAGES)
        .saturating_add(1);
    let mut matches: Vec<RepositoryItem> = Vec::new();
    let mut last_has_more = false;
    for offset in 0..REPOSITORY_SEARCH_PAGES {
        let found = repository_page(github, params, first_page.saturating_add(offset)).await?;
        last_has_more = found.has_more;
        matches.extend(found.repositories.into_iter().filter(|repository| {
            caseless::default_case_fold_str(&repository.full_name)
                .contains(normalized_query.as_str())
        }));
        if offset.saturating_add(1) == REPOSITORY_SEARCH_PAGES || !last_has_more {
            break;
        }
    }
    Ok(RepositoryPage {
        repositories: matches,
        has_more: last_has_more,
    })
}

pub async fn test_repositories<T>(github: &GitHub<T>, repos: &[String]) -> Result<(), SourceError>
where
    T: Transport,
{
    for repo in repos {
        github
            .request(Method::GET, &format!("repos/{repo}"), &[], None)
            .await?;
        let params = [
            ("per_page".to_owned(), "1".to_owned()),
            ("state".to_owned(), "closed".to_owned()),
        ];
        github
            .request(Method::GET, &format!("repos/{repo}/pulls"), &params, None)
            .await?;
    }
    Ok(())
}
