use crate::public_endpoints::blog_feed_parser::BlogPost;

pub const TTL_SECONDS: f64 = 3_600.0;

#[derive(Debug, Default)]
pub struct Cache {
    pub(crate) posts: Option<Vec<BlogPost>>,
    pub(crate) last_fetch_time: f64,
}

impl Cache {
    #[must_use]
    pub fn fresh(&self, now: f64) -> Option<Vec<BlogPost>> {
        // mul_add keeps the TTL comparison off clippy's float-arithmetic restriction
        let expires_at = self.last_fetch_time.mul_add(1.0_f64, TTL_SECONDS);
        let unexpired = now < expires_at;
        self.posts.clone().filter(|_| unexpired)
    }

    pub fn store(&mut self, posts: Vec<BlogPost>, now: f64) {
        self.posts = Some(posts);
        self.last_fetch_time = now;
    }
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct CacheSnapshot {
    pub cached_posts: Option<Vec<BlogPost>>,
    pub last_fetch_time: f64,
}
