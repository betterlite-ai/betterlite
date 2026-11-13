use crate::image::settings::Settings;

pub(crate) const DEFAULT: Settings = Settings {
    default_width: 300,
    default_height: 300,
    default_token_count: 250,
    max_short_side: 768,
    max_long_side: 2000,
    tile_width: 512,
    tile_height: 512,
};

pub(crate) const CUSTOM_DEFAULTS: Settings = Settings {
    default_width: 640,
    default_height: 480,
    default_token_count: 250,
    max_short_side: 768,
    max_long_side: 2000,
    tile_width: 512,
    tile_height: 512,
};

pub(crate) const DEFAULT_COUNT_123: Settings = Settings {
    default_width: 300,
    default_height: 300,
    default_token_count: 123,
    max_short_side: 768,
    max_long_side: 2000,
    tile_width: 512,
    tile_height: 512,
};

pub(crate) const SMALL_LIMITS: Settings = Settings {
    default_width: 300,
    default_height: 300,
    default_token_count: 250,
    max_short_side: 100,
    max_long_side: 400,
    tile_width: 50,
    tile_height: 50,
};

pub(crate) const RECT_TILES: Settings = Settings {
    default_width: 300,
    default_height: 300,
    default_token_count: 250,
    max_short_side: 768,
    max_long_side: 2000,
    tile_width: 100,
    tile_height: 50,
};

pub(crate) const ZERO_TILE: Settings = Settings {
    default_width: 300,
    default_height: 300,
    default_token_count: 250,
    max_short_side: 768,
    max_long_side: 2000,
    tile_width: 0,
    tile_height: 512,
};

pub(crate) const PNG_1X1_URL: &str = "data:image/x;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAA=";
pub(crate) const PNG_4096_URL: &str =
    "data:image/x;base64,iVBORw0KGgoAAAANSUhEUgAAEAAAABAACAYAAAA=";
pub(crate) const PNG_ZERO_WIDTH_URL: &str = "data:image/x;base64,iVBORw0KGgoAAAANSUhEUgAAAAAAAAPo";
pub(crate) const PNG_TRUNCATED_URL: &str = "data:image/x;base64,iVBORw0KGgo=";
pub(crate) const GIF_URL: &str = "data:image/x;base64,R0lGODlhBwAJAA==";
pub(crate) const JPEG_URL: &str = "data:image/x;base64,/9j/4AAHSkZJRgD/wAARCAANAAs=";
pub(crate) const WEBP_VP8X_URL: &str =
    "data:image/x;base64,UklGRhYAAABXRUJQVlA4WAoAAAAAAAAAKwEAxwAA";
pub(crate) const WEBP_VP8X_WIDE_URL: &str =
    "data:image/x;base64,UklGRhYAAABXRUJQVlA4WAoAAAAAAAAA5wMA8wEA";
pub(crate) const HEIC_URL: &str = "data:image/x;base64,AAAAGGZ0eXBoZWljAAAAAA==";
pub(crate) const UNKNOWN_URL: &str = "data:image/x;base64,bm90IGFuIGltYWdl";
pub(crate) const BAD_BASE64_URL: &str = "data:image/x;base64,!!!!";
pub(crate) const NO_PREFIX_URL: &str = "not a data url";
