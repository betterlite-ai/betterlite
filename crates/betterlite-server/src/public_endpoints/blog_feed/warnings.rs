const EMPTY_POSTS_MESSAGE: &str =
    "LiteLLM: Parsed RSS feed has no valid posts. Falling back to local backup.";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FeedWarning {
    pub subject: Option<String>,
    pub message: String,
}

impl FeedWarning {
    #[must_use]
    pub fn empty_posts() -> Self {
        Self {
            subject: None,
            message: EMPTY_POSTS_MESSAGE.to_owned(),
        }
    }
}

pub trait WarningSink: Send + Sync {
    fn warn(&self, warning: &FeedWarning);
}

pub fn validate_blog_posts<Post>(posts: &[Post]) -> Result<(), FeedWarning> {
    if posts.is_empty() {
        return Err(FeedWarning::empty_posts());
    }
    Ok(())
}

pub struct WarnLog;

impl WarningSink for WarnLog {
    fn warn(&self, warning: &FeedWarning) {
        tracing::warn!(
            subject = warning.subject.as_deref().unwrap_or("None"),
            "{}",
            warning.message
        );
    }
}
