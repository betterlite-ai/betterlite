use async_trait::async_trait;

use super::source::{BudgetPageError, PAGE_SIZE, Page};

mod tests;

// LiteLLM's data_fetch_function plus set_metrics_function, one value so the walk is tested without a database
#[async_trait]
pub trait EntityPageWalk: Send + Sync {
    type Row: Send + Sync;

    async fn fetch(&self, page_size: u64, page: u64) -> Result<Page<Self::Row>, BudgetPageError>;

    fn apply(&self, rows: Vec<Self::Row>) -> Result<(), BudgetPageError>;
}

// LiteLLM's _initialize_budget_metrics: page 1 first, then every page up to the count, and one entity's failure never ends the refresh
pub async fn initialize_entity<Walk>(walk: &Walk, data_type: &'static str)
where
    Walk: EntityPageWalk,
{
    if let Err(error) = walk_pages(walk).await {
        tracing::error!("Error initializing {data_type} budget metrics: {error}");
    }
}

async fn walk_pages<Walk>(walk: &Walk) -> Result<(), BudgetPageError>
where
    Walk: EntityPageWalk,
{
    let first = walk.fetch(PAGE_SIZE, 1).await?;
    let total = first
        .total_count
        .unwrap_or(u64::try_from(first.rows.len()).unwrap_or(u64::MAX));
    let total_pages = total.div_ceil(PAGE_SIZE);
    walk.apply(first.rows)?;
    for page in 2..=total_pages {
        let fetched = walk.fetch(PAGE_SIZE, page).await?;
        walk.apply(fetched.rows)?;
    }
    Ok(())
}
