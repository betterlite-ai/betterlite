pub struct VideoProfile {
    pub resolutions: &'static [&'static str],
    pub resolution_tiers: &'static [(u128, &'static str)],
    pub default_resolution: &'static str,
    pub integer_duration: bool,
    pub reference_key: &'static str,
    pub reference_as_list: bool,
}

pub const SEEDANCE_PROFILE: VideoProfile = VideoProfile {
    resolutions: &["480p", "720p", "1080p", "4k"],
    resolution_tiers: &[
        (480, "480p"),
        (720, "720p"),
        (1080, "1080p"),
        (u128::MAX, "4k"),
    ],
    default_resolution: "720p",
    integer_duration: false,
    reference_key: "image_url",
    reference_as_list: false,
};

pub const H3_PROFILE: VideoProfile = VideoProfile {
    resolutions: &["480P", "768P", "2K", "4K"],
    resolution_tiers: &[
        (480, "480P"),
        (768, "768P"),
        (1440, "2K"),
        (u128::MAX, "4K"),
    ],
    default_resolution: "2K",
    integer_duration: true,
    reference_key: "reference_image_urls",
    reference_as_list: true,
};

#[must_use]
pub fn profile_for_model(model: &str) -> &'static VideoProfile {
    if model.starts_with("minimax/h3/") {
        &H3_PROFILE
    } else {
        &SEEDANCE_PROFILE
    }
}

#[must_use]
pub fn resolution_for_short_side(short_side: u128, profile: &VideoProfile) -> &'static str {
    let mut chosen = profile.default_resolution;
    for &(threshold, resolution) in profile.resolution_tiers {
        chosen = resolution;
        if short_side <= threshold {
            break;
        }
    }
    chosen
}
