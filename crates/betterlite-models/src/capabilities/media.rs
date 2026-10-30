use crate::capabilities::capability_struct;
use serde::{Deserialize, Serialize};

capability_struct! {
    #[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
    #[serde(default)]
    pub struct MediaCapabilities {
        supports_audio_input: "supports_audio_input",
        supports_audio_output: "supports_audio_output",
        supports_vision: "supports_vision",
        supports_image_input: "supports_image_input",
        supports_video_input: "supports_video_input",
        supports_pdf_input: "supports_pdf_input",
        supports_embedding_image_input: "supports_embedding_image_input",
        supports_multimodal: "supports_multimodal",
        supports_image_size: "supports_image_size",
        supports_nova_canvas_edit: "supports_nova_canvas_image_edit",
        gemini_audio_only_live: "gemini_audio_only_live",
        gemini_native_audio: "gemini_native_audio",
        uses_embed_content: "uses_embed_content",
    }
}
