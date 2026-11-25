use betterlite_models::model_registration::JsonObject;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PaginatedModels {
    pub data: Vec<JsonObject>,
    pub total_count: i64,
    pub current_page: i64,
    pub total_pages: i64,
    pub size: i64,
}

// _paginate_models_response: skip=(page-1)*size, ceiling pages for a positive count, and Python's slice bounds.
#[must_use]
pub fn paginate_models(
    models: &[JsonObject],
    page: i64,
    size: i64,
    total_count: Option<i64>,
    search: Option<&str>,
) -> PaginatedModels {
    let total_count =
        total_count.unwrap_or_else(|| i64::try_from(models.len()).unwrap_or(i64::MAX));
    let skip = page.saturating_sub(1).saturating_mul(size);
    let total_pages = if total_count > 0 && size > 0 {
        total_count
            .saturating_add(size.saturating_sub(1))
            .checked_div(size)
            .unwrap_or(0)
    } else {
        0
    };
    tracing::debug!(
        "Pagination: skip={skip}, take={size}, total_count={total_count}, total_pages={total_pages}, search={}",
        search.unwrap_or("None")
    );
    PaginatedModels {
        data: slice_page(models, skip, size),
        total_count,
        current_page: page,
        total_pages,
        size,
    }
}

fn slice_page(models: &[JsonObject], skip: i64, size: i64) -> Vec<JsonObject> {
    let length = i64::try_from(models.len()).unwrap_or(i64::MAX);
    let start = slice_bound(skip, length);
    let stop = slice_bound(skip.saturating_add(size), length);
    let (Ok(start), Ok(stop)) = (usize::try_from(start), usize::try_from(stop)) else {
        return Vec::new();
    };
    if stop <= start {
        return Vec::new();
    }
    models
        .get(start..stop)
        .map_or_else(Vec::new, <[JsonObject]>::to_vec)
}

fn slice_bound(index: i64, length: i64) -> i64 {
    if index < 0 {
        length.saturating_add(index).max(0)
    } else {
        index.min(length)
    }
}
