use super::BlogPostsResponse;
use super::error::LocalPostsError;
use crate::public_endpoints::blog_feed_parser::BlogPost;

const BUNDLED_BLOG_POSTS: &str = include_str!("assets/blog_posts.json");

pub trait LocalPosts: Send + Sync {
    fn load(&self) -> Result<Vec<BlogPost>, LocalPostsError>;
}

pub struct BundledBlogPosts;

impl LocalPosts for BundledBlogPosts {
    fn load(&self) -> Result<Vec<BlogPost>, LocalPostsError> {
        let response: BlogPostsResponse = serde_json::from_str(BUNDLED_BLOG_POSTS)?;
        Ok(response.posts)
    }
}
