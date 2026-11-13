import json
from typing import Literal, Protocol, TypeAlias, TypedDict, cast
from unittest.mock import patch

import httpx
import litellm
from litellm.llms.vertex_ai.text_to_speech.transformation import (
    VertexAITextToSpeechConfig,
)
from litellm.types.llms.vertex_ai import VERTEX_CREDENTIALS_TYPES
from pydantic import JsonValue, TypeAdapter

# an unmapped model prints a provider banner on stdout, which carries the oracle's JSON line
litellm.suppress_debug_info = True

FIXED_PROJECT = "project-1"
UPSTREAM_STATUS = 500
SNIFF_PREFIX = b"X"
SNIFF_TYPE = "audio/x-test"
MEDIA_TYPE_PATH = (
    "litellm.llms.vertex_ai.text_to_speech.transformation.speech_media_type_from_audio_bytes"
)

MESSAGES: dict[str, str] = {
    "SSML input is required when use_ssml is True.": "ssml_required",
    "Either 'text' or 'ssml' must be provided.": "no_input",
    "Only one of 'text' or 'ssml' should be provided, not both.": "both_inputs",
}
VALUE_ERROR = "value_error"

Voice: TypeAlias = str | dict[str, JsonValue] | None
Params: TypeAlias = dict[str, JsonValue]


class RequestData(TypedDict, total=False):
    dict_body: dict[str, JsonValue]


class BinaryResponse(Protocol):
    response: httpx.Response
    content: bytes


class SpeechConfig(Protocol):
    def map_openai_params(
        self, model: str, optional_params: Params, voice: Voice = None, *, kwargs: Params
    ) -> tuple[str | None, Params]: ...

    def transform_text_to_speech_request(self, **kwargs: JsonValue) -> RequestData: ...

    def transform_text_to_speech_response(
        self, model: str, raw_response: httpx.Response, logging_obj: JsonValue
    ) -> BinaryResponse: ...


CONFIG = cast(  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/vertex_ai/text_to_speech/transformation.py#L219-L484
    SpeechConfig, VertexAITextToSpeechConfig()
)


class AlloyVoiceInput(TypedDict):
    function: Literal["alloy_voice"]
    voices: list[Voice]


class CustomVoiceLanguageInput(TypedDict):
    function: Literal["custom_voice_language"]
    voices: list[str]


class AacMapsMp3Input(TypedDict):
    function: Literal["aac_maps_mp3"]
    formats: list[str | None]


class SsmlCase(TypedDict):
    input: str
    use_ssml: bool


class SsmlSubstringInput(TypedDict):
    function: Literal["ssml_substring"]
    cases: list[SsmlCase]


class AudioCase(TypedDict, total=False):
    response_format: str
    speed: int
    audio_config: dict[str, JsonValue]


class FullAudioConfigInput(TypedDict):
    function: Literal["full_audio_config"]
    cases: list[AudioCase]


class VoiceDictCase(TypedDict, total=False):
    litellm_voice: dict[str, JsonValue]
    mapped_voice: dict[str, JsonValue]
    voice: str


class VoiceDictPrecedenceInput(TypedDict):
    function: Literal["voice_dict_precedence"]
    cases: list[VoiceDictCase]


class DecodedStatus200Input(TypedDict):
    function: Literal["decoded_status_200"]
    bodies: list[str]


Input: TypeAlias = (
    AlloyVoiceInput
    | CustomVoiceLanguageInput
    | AacMapsMp3Input
    | SsmlSubstringInput
    | FullAudioConfigInput
    | VoiceDictPrecedenceInput
    | DecodedStatus200Input
)


class RequestBody(TypedDict):
    input: dict[str, JsonValue]
    voice: dict[str, JsonValue]
    audioConfig: dict[str, JsonValue]


MAPPED = TypeAdapter(Params)
BODY = TypeAdapter(RequestBody)


def fixed_token(
    self: VertexAITextToSpeechConfig,
    credentials: VERTEX_CREDENTIALS_TYPES | None,
    project_id: str | None,
    *,
    _retry_reauth: bool = False,
) -> tuple[str, str]:
    del self, credentials
    return "oracle-token", FIXED_PROJECT if project_id is None else project_id


def sniff(audio: bytes) -> str | None:
    return SNIFF_TYPE if audio[:1] == SNIFF_PREFIX else None


def map_params(
    voice: Voice, optional_params: Params, kwargs: Params
) -> tuple[str | None, Params]:
    mapped_voice, mapped_params = CONFIG.map_openai_params(
        "m", optional_params, voice=voice, kwargs=kwargs
    )
    return mapped_voice, MAPPED.validate_json(json.dumps(mapped_params))


def transform(
    input_value: str,
    voice: str | None,
    optional_params: Params,
    litellm_params: Params,
) -> RequestBody:
    request_data = CONFIG.transform_text_to_speech_request(
        model="m",
        input=input_value,
        voice=voice,
        optional_params=optional_params,
        litellm_params=litellm_params,
        headers={},
    )
    return BODY.validate_json(json.dumps(request_data.get("dict_body")))


def error_code(error: str) -> str:
    try:
        return MESSAGES[error]
    except KeyError:
        return "other"


def alloy_voice(voices: list[Voice]) -> JsonValue:
    views: list[JsonValue] = []
    for voice in voices:
        mapped_voice, mapped_params = map_params(voice, {}, {})
        views.append({"voice": mapped_voice, "dict": mapped_params.get("vertex_voice_dict")})
    return {"voices": views}


def custom_voice_language(voices: list[str]) -> JsonValue:
    views: list[JsonValue] = []
    for voice in voices:
        _, mapped_params = map_params(voice, {}, {})
        views.append(mapped_params.get("vertex_voice_dict"))
    return {"voices": views}


def aac_maps_mp3(formats: list[str | None]) -> JsonValue:
    encodings: list[JsonValue] = []
    for format_name in formats:
        optional_params: Params = {}
        if format_name is not None:
            optional_params["response_format"] = format_name
        _, mapped_params = map_params(None, optional_params, {})
        encodings.append(mapped_params.get("audioEncoding"))
    return {"encodings": encodings}


def ssml_substring(cases: list[SsmlCase]) -> JsonValue:
    with patch.object(VertexAITextToSpeechConfig, "get_access_token", fixed_token):
        inputs: list[JsonValue] = []
        for case in cases:
            _, mapped_params = map_params(None, {}, {"use_ssml": case["use_ssml"]})
            try:
                body = transform(case["input"], None, mapped_params, {})
            except ValueError as error:
                inputs.append({"error": error_code(str(error))})
            else:
                inputs.append({"input": body["input"]})
    return {"inputs": inputs}


def full_audio_config(cases: list[AudioCase]) -> JsonValue:
    with patch.object(VertexAITextToSpeechConfig, "get_access_token", fixed_token):
        configs: list[JsonValue] = []
        for case in cases:
            optional_params: Params = {}
            response_format = case.get("response_format")
            if response_format is not None:
                optional_params["response_format"] = response_format
            speed = case.get("speed")
            if speed is not None:
                optional_params["speed"] = speed
            kwargs: Params = {}
            audio_config = case.get("audio_config")
            if audio_config is not None:
                kwargs["audioConfig"] = audio_config
            _, mapped_params = map_params(None, optional_params, kwargs)
            body = transform("hi", None, mapped_params, {})
            configs.append(body["audioConfig"])
    return {"configs": configs}


def voice_dict_precedence(cases: list[VoiceDictCase]) -> JsonValue:
    with patch.object(VertexAITextToSpeechConfig, "get_access_token", fixed_token):
        voices: list[JsonValue] = []
        for case in cases:
            voice = case.get("voice")
            _, optional_params = map_params(voice, {}, {})
            mapped_voice = case.get("mapped_voice")
            if mapped_voice is not None:
                optional_params["vertex_voice_dict"] = mapped_voice
            litellm_params: Params = {}
            litellm_voice = case.get("litellm_voice")
            if litellm_voice is not None:
                litellm_params["vertex_voice_dict"] = litellm_voice
            try:
                body = transform("hi", voice, optional_params, litellm_params)
            except ValueError as error:
                voices.append({"error": error_code(str(error))})
            else:
                voices.append(body["voice"])
    return {"voices": voices}


def response_view(body: str) -> JsonValue:
    raw = httpx.Response(status_code=UPSTREAM_STATUS, content=body.encode())
    try:
        result = CONFIG.transform_text_to_speech_response("m", raw, None)
    except json.JSONDecodeError:
        return {"error": "json"}
    except AttributeError:
        return {"error": "not_object"}
    except TypeError:
        return {"error": "not_text"}
    except ValueError:
        return {"error": VALUE_ERROR}
    return {
        "status": result.response.status_code,
        "contentType": result.response.headers.get("content-type"),
        "bytes": list(result.content),
    }


def decoded_status_200(bodies: list[str]) -> JsonValue:
    with patch(MEDIA_TYPE_PATH, sniff):
        return {"bodies": [response_view(body) for body in bodies]}


def run(input_value: Input) -> JsonValue:
    result: JsonValue
    match input_value["function"]:
        case "alloy_voice":
            result = alloy_voice(input_value["voices"])
        case "custom_voice_language":
            result = custom_voice_language(input_value["voices"])
        case "aac_maps_mp3":
            result = aac_maps_mp3(input_value["formats"])
        case "ssml_substring":
            result = ssml_substring(input_value["cases"])
        case "full_audio_config":
            result = full_audio_config(input_value["cases"])
        case "voice_dict_precedence":
            result = voice_dict_precedence(input_value["cases"])
        case "decoded_status_200":
            result = decoded_status_200(input_value["bodies"])
    return result
