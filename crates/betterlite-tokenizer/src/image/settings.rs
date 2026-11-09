use serde::Deserialize;

const DEFAULT_IMAGE_WIDTH: u32 = 300;
const DEFAULT_IMAGE_HEIGHT: u32 = 300;
const DEFAULT_IMAGE_TOKEN_COUNT: u64 = 250;
const MAX_HIGH_RES_SHORT_SIDE: u32 = 768;
const MAX_HIGH_RES_LONG_SIDE: u32 = 2000;
const DEFAULT_TILE_WIDTH: u32 = 512;
const DEFAULT_TILE_HEIGHT: u32 = 512;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Settings {
    pub default_width: u32,
    pub default_height: u32,
    pub default_token_count: u64,
    pub max_short_side: u32,
    pub max_long_side: u32,
    pub tile_width: u32,
    pub tile_height: u32,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            default_width: DEFAULT_IMAGE_WIDTH,
            default_height: DEFAULT_IMAGE_HEIGHT,
            default_token_count: DEFAULT_IMAGE_TOKEN_COUNT,
            max_short_side: MAX_HIGH_RES_SHORT_SIDE,
            max_long_side: MAX_HIGH_RES_LONG_SIDE,
            tile_width: DEFAULT_TILE_WIDTH,
            tile_height: DEFAULT_TILE_HEIGHT,
        }
    }
}
