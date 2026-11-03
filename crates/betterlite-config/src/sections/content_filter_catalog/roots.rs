pub const DATA_DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/assets/content_filter");
pub const CATEGORIES_DIR: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/assets/content_filter/categories"
);
pub const POLICY_TEMPLATES_DIR: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/assets/content_filter/policy_templates"
);
// betterlite ships no legacy layout, so LEGACY_DATA_DIR only keeps DATA_ROOTS lined up with the ported tuple.
pub const LEGACY_DATA_DIR: &str =
    concat!(env!("CARGO_MANIFEST_DIR"), "/assets/content_filter_legacy");

pub const DATA_ROOTS: [&str; 2] = [DATA_DIR, LEGACY_DATA_DIR];

mod tests;
