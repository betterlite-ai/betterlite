use std::collections::BTreeMap;

use betterlite_prompts::arguments::json::Json;
use indexmap::IndexMap;
use rstest::rstest;
use secrecy::SecretString;

use crate::native_speech::endpoint::get_complete_url;
use crate::native_speech::headers::validate_environment;
use crate::native_speech::identity::NativeSpeechIdentity;
use crate::native_speech::mapping::{SpeechKwargs, SpeechOptionalParams, convert_speed_to_rate};
use crate::native_speech::request::{SpeechTransform, transform_request};
use crate::native_speech::response::{HttpxBinaryResponseContent, RawSpeechResponse};

use super::support;

const SPEEDS: [Option<f64>; 10] = [
    None,
    Some(1.0),
    Some(2.0),
    Some(0.5),
    Some(0.25),
    Some(4.0),
    Some(5.0),
    Some(0.125),
    Some(1.0625),
    Some(0.0625),
];

const RATES: [Option<&str>; 10] = [
    None,
    Some("+0%"),
    Some("+100%"),
    Some("-50%"),
    Some("-75%"),
    Some("+300%"),
    Some("+400%"),
    Some("-87%"),
    Some("+6%"),
    Some("-93%"),
];

const VOICE_LANGUAGE_CASES: [(Option<&str>, Option<&str>, &str); 6] = [
    (None, None, "<voice name='en-US-AriaNeural'>"),
    (Some("alloy"), None, "<voice name='en-US-JennyNeural'>"),
    (Some("fr-FR"), None, "<voice name='fr-FR'>"),
    (
        Some("en-US-AriaNeural"),
        Some("es-ES"),
        "<voice name='en-US-AriaNeural' xml:lang='es-ES'>",
    ),
    (Some("alloy"), Some(""), "<voice name='en-US-JennyNeural'>"),
    (
        None,
        Some("fr-FR"),
        "<voice name='en-US-AriaNeural' xml:lang='fr-FR'>",
    ),
];

const ENDPOINT_CASES: [(Option<&str>, &str); 13] = [
    (
        Some("https://eastus.api.cognitive.microsoft.com"),
        "https://eastus.tts.speech.microsoft.com/cognitiveservices/v1",
    ),
    (
        Some("https://api.cognitive.microsoft.com/"),
        "https://tts.speech.microsoft.com/cognitiveservices/v1",
    ),
    (
        Some("https://eastus.api.cognitive.microsoft.com/openai/deployments/x?api-version=1"),
        "https://eastus.tts.speech.microsoft.com/cognitiveservices/v1",
    ),
    (
        Some("https://eastus.tts.speech.microsoft.com"),
        "https://eastus.tts.speech.microsoft.com/cognitiveservices/v1",
    ),
    (
        Some("https://eastus.tts.speech.microsoft.com/cognitiveservices/v1"),
        "https://eastus.tts.speech.microsoft.com/cognitiveservices/v1",
    ),
    (
        Some("https://eastus.tts.speech.microsoft.com/cognitiveservices/v1/"),
        "https://eastus.tts.speech.microsoft.com/cognitiveservices/v1",
    ),
    (
        Some("https://custom.example/audio"),
        "https://custom.example/audio/cognitiveservices/v1",
    ),
    (
        Some("https://custom.example/audio/"),
        "https://custom.example/audio/cognitiveservices/v1",
    ),
    (
        Some("eastus.api.cognitive.microsoft.com"),
        "eastus.api.cognitive.microsoft.com/cognitiveservices/v1",
    ),
    (
        Some("https://eastus.api.cognitive.microsoft.com.evil.example"),
        "https://eastus.api.cognitive.microsoft.com.evil.example/cognitiveservices/v1",
    ),
    (
        Some("https://TTS.Speech.Microsoft.com/cognitiveservices/v1"),
        "https://TTS.Speech.Microsoft.com/cognitiveservices/v1",
    ),
    (
        Some("https://eastus.api.cognitive.microsoft.com:badport"),
        "https://eastus.api.cognitive.microsoft.com:badport/cognitiveservices/v1",
    ),
    (Some(""), "/cognitiveservices/v1"),
];

const FORMAT_CASES: [(Option<&str>, &str); 9] = [
    (None, "audio-24khz-48kbitrate-mono-mp3"),
    (Some("mp3"), "audio-24khz-48kbitrate-mono-mp3"),
    (Some("aac"), "audio-24khz-48kbitrate-mono-mp3"),
    (Some("flac"), "audio-24khz-48kbitrate-mono-mp3"),
    (Some("opus"), "ogg-48khz-16bit-mono-opus"),
    (Some("wav"), "riff-24khz-16bit-mono-pcm"),
    (Some("pcm"), "raw-24khz-16bit-mono-pcm"),
    (Some("mp4"), "mp4"),
    (Some(""), ""),
];

const VOICE_CASES: [(Option<&str>, Option<&str>, &str); 9] = [
    (None, None, "en-US-AriaNeural"),
    (
        Some("alloy"),
        Some("en-US-JennyNeural"),
        "en-US-JennyNeural",
    ),
    (Some("echo"), Some("en-US-GuyNeural"), "en-US-GuyNeural"),
    (Some("fable"), Some("en-GB-RyanNeural"), "en-GB-RyanNeural"),
    (Some("onyx"), Some("en-US-DavisNeural"), "en-US-DavisNeural"),
    (Some("nova"), Some("en-US-AmberNeural"), "en-US-AmberNeural"),
    (
        Some("shimmer"),
        Some("en-US-AriaNeural"),
        "en-US-AriaNeural",
    ),
    (
        Some("en-GB-SoniaNeural"),
        Some("en-GB-SoniaNeural"),
        "en-GB-SoniaNeural",
    ),
    (Some(""), Some(""), "en-US-AriaNeural"),
];

#[rstest]
#[case::plain_text_ssml_escaping(0)]
#[case::speed_rate_boundaries(1)]
#[case::voice_language(2)]
#[case::style_role_element(3)]
#[case::existing_ssml(4)]
#[case::cognitive_to_tts_endpoint(5)]
#[case::response_format_header(6)]
#[case::binary_native_speech(7)]
#[case::native_voice_map(8)]
#[case::native_speech_headers(9)]
fn native_speech_cases(#[case] case: u8) {
    match case {
        0 => plain_text_ssml_escaping(),
        1 => speed_rate_boundaries(),
        2 => voice_language(),
        3 => style_role_element(),
        4 => existing_ssml(),
        5 => cognitive_to_tts_endpoint(),
        6 => response_format_header(),
        7 => binary_native_speech(),
        8 => native_voice_map(),
        _ => native_speech_headers(),
    }
}

fn plain_text_ssml_escaping() {
    let (_, mapped) = support::map(
        None,
        SpeechOptionalParams::default(),
        SpeechKwargs::default(),
    );
    let escaped = support::data("a & b < c > d \" e ' f", None, &mapped);
    assert_eq!(
        escaped.ssml_body,
        "<speak version='1.0' xmlns='http://www.w3.org/2001/10/synthesis' xml:lang='en-US'>
    <voice name='en-US-AriaNeural'>
        <prosody rate='0%'>a &amp; b &lt; c &gt; d &quot; e &apos; f</prosody>
    </voice>
</speak>"
    );
    let pre_escaped = support::data("&lt;tag&gt;", None, &mapped);
    assert!(
        pre_escaped
            .ssml_body
            .contains("<prosody rate='0%'>&amp;lt;tag&amp;gt;</prosody>")
    );
    let empty = support::data("", None, &mapped);
    assert!(empty.ssml_body.contains("<prosody rate='0%'></prosody>"));
}

fn speed_views(speeds: &[Option<f64>]) -> Vec<(Option<String>, String)> {
    let mut views = Vec::with_capacity(speeds.len());
    for speed in speeds {
        let optional = SpeechOptionalParams {
            response_format: None,
            speed: *speed,
        };
        let (_, mapped) = support::map(None, optional, SpeechKwargs::default());
        let body = support::data("hi", None, &mapped).ssml_body;
        views.push((mapped.rate, body));
    }
    views
}

fn speed_rate_boundaries() {
    let views = speed_views(&SPEEDS);
    let rates: Vec<Option<&str>> = views.iter().map(|(rate, _)| rate.as_deref()).collect();
    assert_eq!(rates, RATES);
    assert!(views[0].1.contains("<prosody rate='0%'>"));
    assert!(views[1].1.contains("<prosody rate='+0%'>"));
    assert!(views[3].1.contains("<prosody rate='-50%'>"));
    assert!(views[7].1.contains("<prosody rate='-87%'>"));
    for speed in [f64::INFINITY, f64::NAN, f64::MAX] {
        assert!(convert_speed_to_rate(speed).is_err());
    }
}

fn voice_language() {
    for (voice, lang, expected) in VOICE_LANGUAGE_CASES {
        let spelled = voice.map(str::to_owned).map(Json::Text);
        let kwargs = SpeechKwargs {
            style: None,
            styledegree: None,
            role: None,
            lang: lang.map(str::to_owned),
        };
        let (name, mapped) =
            support::map(spelled.as_ref(), SpeechOptionalParams::default(), kwargs);
        let body = support::data("hi", name.as_deref(), &mapped).ssml_body;
        assert!(body.contains(expected), "{body}");
        assert!(body.contains("<speak version='1.0'"));
        assert!(body.contains("xml:lang='en-US'"));
    }
}

fn style_role_element() {
    style_matrix();
    style_literals();
}

fn style_matrix() {
    let cases = [
        (None, None, None, false),
        (Some("cheerful"), None, None, true),
        (None, Some("1.5"), None, true),
        (None, None, Some("Girl"), true),
        (Some("cheerful"), Some("0.5"), Some("Boy"), true),
        (Some(""), Some(""), Some(""), false),
        (Some("sad"), Some(""), None, true),
    ];
    for (style, styledegree, role, wrapped) in cases {
        let kwargs = SpeechKwargs {
            style: style.map(str::to_owned),
            styledegree: styledegree.map(str::to_owned),
            role: role.map(str::to_owned),
            lang: None,
        };
        let (_, mapped) = support::map(None, SpeechOptionalParams::default(), kwargs);
        let body = support::data("x<y", None, &mapped).ssml_body;
        assert!(body.contains("<prosody rate='0%'>x&lt;y</prosody>"));
        assert_eq!(body.contains("<mstts:express-as"), wrapped);
        assert_eq!(
            body.contains("xmlns:mstts='https://www.w3.org/2001/mstts'"),
            wrapped
        );
    }
}

fn style_literals() {
    let ordered = SpeechKwargs {
        style: Some("cheerful".to_owned()),
        styledegree: Some("0.5".to_owned()),
        role: Some("Boy".to_owned()),
        lang: Some("es-ES".to_owned()),
    };
    let (_, mapped) = support::map(None, SpeechOptionalParams::default(), ordered);
    let body = support::data("plain & text", None, &mapped).ssml_body;
    assert!(body.contains(
        "<mstts:express-as style='cheerful' styledegree='0.5' role='Boy'><prosody rate='0%'>plain &amp; text</prosody></mstts:express-as>"
    ));
    let literal = SpeechKwargs {
        style: Some("a&b\"c".to_owned()),
        styledegree: None,
        role: Some("Senior_Male".to_owned()),
        lang: None,
    };
    let (_, mapped) = support::map(None, SpeechOptionalParams::default(), literal);
    let escaped_text = support::data("x<y", None, &mapped).ssml_body;
    assert!(escaped_text.contains("<mstts:express-as style='a&b\"c' role='Senior_Male'>"));
}

fn existing_ssml() {
    let (_, mapped) = support::map(
        None,
        SpeechOptionalParams::default(),
        SpeechKwargs::default(),
    );
    let passthrough = [
        "<speak version='1.0' xml:lang='en-US'><voice name='en-US-AriaNeural'>Hi</voice></speak>",
        "prefix <speak>inner</speak> suffix",
        "prefix <speak version='1.0'>inner</speak>",
        "<speak >not closed",
    ];
    for text in passthrough {
        let data = support::data(text, None, &mapped);
        assert_eq!(data.ssml_body, text);
        assert_eq!(
            data.headers
                .get("X-Microsoft-OutputFormat")
                .map(String::as_str),
            Some("audio-24khz-48kbitrate-mono-mp3")
        );
    }
    let generated = support::data("<SPEAK>x</SPEAK><speakx>y</speakx>", None, &mapped);
    assert!(
        generated
            .ssml_body
            .contains("&lt;SPEAK&gt;x&lt;/SPEAK&gt;&lt;speakx&gt;y&lt;/speakx&gt;")
    );
    let wav = SpeechOptionalParams {
        response_format: Some("wav".to_owned()),
        speed: None,
    };
    let (_, wav_mapped) = support::map(None, wav, SpeechKwargs::default());
    let wav_data = support::data("<speak>Hi</speak>", None, &wav_mapped);
    assert_eq!(wav_data.ssml_body, "<speak>Hi</speak>");
    assert_eq!(
        wav_data
            .headers
            .get("X-Microsoft-OutputFormat")
            .map(String::as_str),
        Some("riff-24khz-16bit-mono-pcm")
    );
}

fn cognitive_to_tts_endpoint() {
    for (api_base, expected) in ENDPOINT_CASES {
        let identity = NativeSpeechIdentity {
            api_base: api_base.map(str::to_owned),
            api_key: None,
        };
        assert_eq!(get_complete_url(&identity).unwrap(), expected);
    }
    let missing = NativeSpeechIdentity {
        api_base: None,
        api_key: None,
    };
    let error = get_complete_url(&missing).unwrap_err();
    assert!(error.to_string().contains("api.cognitive.microsoft.com"));
    assert!(error.to_string().contains("tts.speech.microsoft.com"));
}

fn response_format_header() {
    for (format, expected) in FORMAT_CASES {
        let optional = SpeechOptionalParams {
            response_format: format.map(str::to_owned),
            speed: None,
        };
        let (_, mapped) = support::map(None, optional, SpeechKwargs::default());
        assert_eq!(mapped.output_format, expected);
        let mut caller = BTreeMap::new();
        caller.insert(
            "X-Microsoft-OutputFormat".to_owned(),
            "caller-format".to_owned(),
        );
        caller.insert("X-Keep".to_owned(), "kept".to_owned());
        let data = transform_request(SpeechTransform {
            input: "hi",
            voice: None,
            mapped: &mapped,
            headers: caller,
        });
        assert_eq!(
            data.headers
                .get("X-Microsoft-OutputFormat")
                .map(String::as_str),
            Some(expected)
        );
        assert_eq!(data.headers.get("X-Keep").map(String::as_str), Some("kept"));
    }
}

fn binary_native_speech() {
    let cases = [
        (200_u16, "ID3audio", Some("audio/mpeg")),
        (400, "", Some("application/json")),
        (500, "error text", Some("text/plain")),
        (204, "звук", None),
    ];
    for (status, body, content_type) in cases {
        let mut headers = BTreeMap::new();
        headers.insert("x-upstream".to_owned(), "kept".to_owned());
        if let Some(value) = content_type {
            headers.insert("content-type".to_owned(), value.to_owned());
        }
        let raw = RawSpeechResponse {
            status,
            headers,
            body: body.as_bytes().to_vec(),
        };
        let wrapped = HttpxBinaryResponseContent::from(raw);
        assert_eq!(wrapped.status, status);
        assert_eq!(wrapped.body, body.as_bytes());
        assert_eq!(
            wrapped.headers.get("x-upstream").map(String::as_str),
            Some("kept")
        );
        assert_eq!(
            wrapped.headers.get("content-type").map(String::as_str),
            content_type
        );
    }
}

fn native_voice_map() {
    for (voice, expected_name, spoken) in VOICE_CASES {
        let spelled = voice.map(str::to_owned).map(Json::Text);
        let (name, mapped) = support::map(
            spelled.as_ref(),
            SpeechOptionalParams::default(),
            SpeechKwargs::default(),
        );
        assert_eq!(name.as_deref(), expected_name);
        let body = support::data("hi", name.as_deref(), &mapped).ssml_body;
        assert!(body.contains(&format!("<voice name='{spoken}'>")));
    }
    non_string_voices();
}

fn non_string_voices() {
    let non_strings = [
        Json::Number(serde_json::Number::from(3_u8)),
        Json::Bool(true),
        Json::List(Vec::new()),
        Json::Object(IndexMap::new()),
    ];
    for voice in &non_strings {
        let (name, mapped) = support::map(
            Some(voice),
            SpeechOptionalParams::default(),
            SpeechKwargs::default(),
        );
        assert!(name.is_none());
        let body = support::data("hi", None, &mapped).ssml_body;
        assert!(body.contains("<voice name='en-US-AriaNeural'>"));
    }
}

fn native_speech_headers() {
    header_precedence();
    header_key_absence();
}

fn header_precedence() {
    let caller = BTreeMap::from([
        (
            "Ocp-Apim-Subscription-Key".to_owned(),
            "caller-key".to_owned(),
        ),
        ("Content-Type".to_owned(), "caller/type".to_owned()),
        ("User-Agent".to_owned(), "caller-agent".to_owned()),
        ("X-Trace".to_owned(), "abc".to_owned()),
    ]);
    let identity = NativeSpeechIdentity {
        api_base: None,
        api_key: Some(SecretString::from("identity-key")),
    };
    let headers = validate_environment(&identity, caller.clone());
    assert_eq!(
        headers.get("Ocp-Apim-Subscription-Key").map(String::as_str),
        Some("identity-key")
    );
    assert_eq!(
        headers.get("Content-Type").map(String::as_str),
        Some("application/ssml+xml")
    );
    assert_eq!(
        headers.get("User-Agent").map(String::as_str),
        Some("litellm")
    );
    assert_eq!(headers.get("X-Trace").map(String::as_str), Some("abc"));
    assert_eq!(
        caller.get("Content-Type").map(String::as_str),
        Some("caller/type")
    );
}

fn header_key_absence() {
    let caller = BTreeMap::from([(
        "Ocp-Apim-Subscription-Key".to_owned(),
        "caller-key".to_owned(),
    )]);
    let empty_key = NativeSpeechIdentity {
        api_base: None,
        api_key: Some(SecretString::from("")),
    };
    assert_eq!(
        validate_environment(&empty_key, caller.clone())
            .get("Ocp-Apim-Subscription-Key")
            .map(String::as_str),
        Some("caller-key")
    );
    let absent_key = NativeSpeechIdentity {
        api_base: None,
        api_key: None,
    };
    let kept = validate_environment(&absent_key, caller);
    assert_eq!(
        kept.get("Ocp-Apim-Subscription-Key").map(String::as_str),
        Some("caller-key")
    );
    assert_eq!(
        kept.get("Content-Type").map(String::as_str),
        Some("application/ssml+xml")
    );
    assert_eq!(kept.len(), 3);
}
