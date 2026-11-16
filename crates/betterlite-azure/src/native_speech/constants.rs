pub const DEFAULT_VOICE: &str = "en-US-AriaNeural";
pub const COGNITIVE_SERVICES_DOMAIN: &str = "api.cognitive.microsoft.com";
pub const TTS_SPEECH_DOMAIN: &str = "tts.speech.microsoft.com";
pub const TTS_ENDPOINT_PATH: &str = "/cognitiveservices/v1";
pub const DEFAULT_OUTPUT_FORMAT: &str = "audio-24khz-48kbitrate-mono-mp3";
pub const DEFAULT_RATE: &str = "0%";
pub const SYNTHESIS_XMLNS: &str = "http://www.w3.org/2001/10/synthesis";
pub const MSTTS_XMLNS: &str = "https://www.w3.org/2001/mstts";

pub const VOICE_MAPPINGS: [(&str, &str); 6] = [
    ("alloy", "en-US-JennyNeural"),
    ("echo", "en-US-GuyNeural"),
    ("fable", "en-GB-RyanNeural"),
    ("onyx", "en-US-DavisNeural"),
    ("nova", "en-US-AmberNeural"),
    ("shimmer", "en-US-AriaNeural"),
];

pub const FORMAT_MAPPINGS: [(&str, &str); 6] = [
    ("mp3", "audio-24khz-48kbitrate-mono-mp3"),
    ("opus", "ogg-48khz-16bit-mono-opus"),
    ("aac", "audio-24khz-48kbitrate-mono-mp3"),
    ("flac", "audio-24khz-48kbitrate-mono-mp3"),
    ("wav", "riff-24khz-16bit-mono-pcm"),
    ("pcm", "raw-24khz-16bit-mono-pcm"),
];
