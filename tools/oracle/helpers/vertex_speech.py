import json
from typing import Literal, Protocol, TypeAlias, TypedDict, cast
from unittest.mock import patch

import httpx
import litellm
from litellm.llms.vertex_ai.text_to_speech.transformation import VertexAITextToSpeechConfig
from litellm.types.llms.vertex_ai import VERTEX_CREDENTIALS_TYPES
from pydantic import JsonValue, TypeAdapter

# an unmapped model prints a provider banner on stdout, which carries the oracle's JSON line
litellm.suppress_debug_info = True

PROJECT = "project-1"
STATUS = 500
PREFIX = b"X"
MEDIA = "litellm.llms.vertex_ai.text_to_speech.transformation.speech_media_type_from_audio_bytes"
Voice: TypeAlias = str | dict[str, JsonValue] | None
Params: TypeAlias = dict[str, JsonValue]
FAILED: JsonValue = {"error": True}


class Reply(Protocol):
    response: httpx.Response
    content: bytes


class Config(Protocol):
    def map_openai_params(
        self, model: str, optional_params: Params, voice: Voice = None, *, kwargs: Params
    ) -> tuple[str | None, Params]: ...

    def transform_text_to_speech_request(self, **kwargs: JsonValue) -> Params: ...

    def transform_text_to_speech_response(
        self, model: str, raw_response: httpx.Response, logging_obj: JsonValue
    ) -> Reply: ...


CONFIG = cast(  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/vertex_ai/text_to_speech/transformation.py#L219-L484
    Config, VertexAITextToSpeechConfig()
)


class Voices(TypedDict):
    function: Literal["alloy_voice", "custom_voice_language"]
    voices: list[Voice]


class Formats(TypedDict):
    function: Literal["aac_maps_mp3"]
    formats: list[str | None]


class Cases(TypedDict):
    function: Literal["ssml_substring", "full_audio_config", "voice_dict_precedence"]
    cases: list[dict[str, JsonValue]]


class Bodies(TypedDict):
    function: Literal["decoded_status_200"]
    bodies: list[str]


Input: TypeAlias = Voices | Formats | Cases | Bodies

MAPPED = TypeAdapter(Params)


def fixed_token(
    self: VertexAITextToSpeechConfig,
    credentials: VERTEX_CREDENTIALS_TYPES | None,
    project_id: str | None,
) -> tuple[str, str]:
    del self, credentials
    return "oracle-token", PROJECT if project_id is None else project_id


def sniff(audio: bytes) -> str | None:
    if audio[:1] == PREFIX:
        return "audio/x-test"
    return None


def mapped(voice: Voice, optional: Params, kwargs: Params) -> Params:
    _, values = CONFIG.map_openai_params("m", optional, voice=voice, kwargs=kwargs)
    return MAPPED.validate_json(json.dumps(values))


def transform(value: str, voice: str | None, optional: Params, params: Params) -> Params:
    data = CONFIG.transform_text_to_speech_request(
        model="m", input=value, voice=voice, optional_params=optional,
        litellm_params=params, headers={},
    )
    return MAPPED.validate_json(json.dumps(data.get("dict_body")))


def alloy_voice(voices: list[Voice]) -> JsonValue:
    return {
        "voices": [{"voice": v, "dict": mapped(v, {}, {}).get("vertex_voice_dict")} for v in voices]
    }


def aac_maps_mp3(formats: list[str | None]) -> JsonValue:
    return {
        "encodings": [
            mapped(None, {} if f is None else {"response_format": f}, {}).get("audioEncoding")
            for f in formats
        ]
    }


def prepared(
    value: str, voice: str | None, optional: Params, params: Params, field: str
) -> JsonValue:
    try:
        return transform(value, voice, optional, params)[field]
    except ValueError:
        return FAILED


def case_view(case: dict[str, JsonValue], field: str) -> JsonValue:
    voice = case.get("voice")
    spoken = None if voice is None else str(voice)
    optional: Params = {k: case[k] for k in ("response_format", "speed") if case.get(k) is not None}
    renamed = (("audio_config", "audioConfig"), ("use_ssml", "use_ssml"))
    kwargs: Params = {name: case[key] for key, name in renamed if case.get(key) is not None}
    params = mapped(spoken, optional, kwargs)
    mapped_voice = case.get("mapped_voice")
    if mapped_voice is not None:
        params["vertex_voice_dict"] = mapped_voice
    litellm_voice = case.get("litellm_voice")
    litellm_params: Params = {} if litellm_voice is None else {"vertex_voice_dict": litellm_voice}
    return prepared(str(case.get("input", "hi")), spoken, params, litellm_params, field)


def response_view(text: str) -> JsonValue:
    raw = httpx.Response(status_code=STATUS, content=text.encode())
    try:
        reply = CONFIG.transform_text_to_speech_response("m", raw, None)
    except (json.JSONDecodeError, AttributeError, TypeError, ValueError):
        return FAILED
    headers = reply.response.headers
    return {
        "status": reply.response.status_code,
        "contentType": next(iter(headers.get_list("content-type")), None),
        "bytes": list(reply.content),
    }


def decoded_status_200(bodies: list[str]) -> JsonValue:
    with patch(MEDIA, sniff):
        return {"bodies": [response_view(body) for body in bodies]}


VIEWS = {
    "ssml_substring": ("inputs", "input"),
    "full_audio_config": ("configs", "audioConfig"),
    "voice_dict_precedence": ("voices", "voice"),
}


def run(input_value: Input) -> JsonValue:
    with patch.object(VertexAITextToSpeechConfig, "get_access_token", fixed_token):
        match input_value["function"]:
            case "alloy_voice":
                return alloy_voice(input_value["voices"])
            case "custom_voice_language":
                voices = [mapped(v, {}, {}).get("vertex_voice_dict") for v in input_value["voices"]]
                return {"voices": voices}
            case "aac_maps_mp3":
                return aac_maps_mp3(input_value["formats"])
            case "ssml_substring" | "full_audio_config" | "voice_dict_precedence":
                key, field = VIEWS[input_value["function"]]
                return {key: [case_view(case, field) for case in input_value["cases"]]}
            case "decoded_status_200":
                return decoded_status_200(input_value["bodies"])
