use std::time::Duration;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use betterlite_openai::api::image_contracts::records::{ImageData, ImageResponse, ImageUsage};

// where the config's response transforms take response_ms from: the sync elapsed time or the async call timing
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum TopazTiming {
    // transform_response_image_variation: raw_response.elapsed.total_seconds() * 1000
    Elapsed(Duration),
    // async_transform_response_image_variation: logging_obj.get_response_ms()
    Measured(f64),
}

impl TopazTiming {
    fn response_ms(self) -> f64 {
        match self {
            Self::Elapsed(elapsed) => elapsed.as_secs_f64().mul_add(1000.0_f64, 0.0_f64),
            Self::Measured(response_ms) => response_ms,
        }
    }
}

// the ImageResponse both entry points build, with the response_ms they compute; LiteLLM's ImageResponse constructor drops it
#[derive(Debug, Clone, PartialEq)]
pub struct TopazReply {
    pub response: ImageResponse,
    pub response_ms: f64,
}

// _common_transform_response_image_variation: base64 of the bytes as received, with a null url, revised prompt and the constructor's zero usage
#[must_use]
pub fn transform_response(body: &[u8], timing: TopazTiming, created: i64) -> TopazReply {
    let data = vec![ImageData {
        b64_json: Some(STANDARD.encode(body)),
        revised_prompt: None,
        url: None,
    }];
    TopazReply {
        response: ImageResponse {
            created,
            data: Some(data),
            usage: Some(ImageUsage::default()),
        },
        response_ms: timing.response_ms(),
    }
}
