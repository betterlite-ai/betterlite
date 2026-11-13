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
TYPE = "audio/x-test"
MEDIA = "litellm.llms.vertex_ai.text_to_speech.transformation.speech_media_type_from_audio_bytes"
ERROR = "value_error"
Voice: TypeAlias = str | dict[str, JsonValue] | None
Params: TypeAlias = dict[str, JsonValue]


class RequestData(TypedDict, total=False):
    dict_body: dict[str, JsonValue]


class Reply(Protocol):
    response: httpx.Response
    content: bytes


class Config(Protocol):
    def map_openai_params(
        self, model: str, optional_params: Params, voice: Voice = None, *, kwargs: Params
    ) -> tuple[str | None, Params]: ...

    def transform_text_to_speech_request(self, **kwargs: JsonValue) -> RequestData: ...

    def transform_text_to_speech_response(
        self, model: str, raw_response: httpx.Response, logging_obj: JsonValue
    ) -> Reply: ...


CONFIG = cast(  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/vertex_ai/text_to_speech/transformation.py#L219-L484
    Config, VertexAITextToSpeechConfig()
)


class AlloyVoiceInput(TypedDict):
    function: Literal["alloy_voice"]
    voices: list[Voice]


class LanguageInput(TypedDict):
    function: Literal["custom_voice_language"]
    voices: list[str]


class FormatInput(TypedDict):
    function: Literal["aac_maps_mp3"]
    formats: list[str | None]


class SsmlInput(TypedDict):
    function: Literal["ssml_substring"]
    cases: list[dict[str, JsonValue]]


class AudioInput(TypedDict):
    function: Literal["full_audio_config"]
    cases: list[dict[str, JsonValue]]


class VoiceInput(TypedDict):
    function: Literal["voice_dict_precedence"]
    cases: list[dict[str, JsonValue]]


class DecodeInput(TypedDict):
    function: Literal["decoded_status_200"]
    bodies: list[str]


Input: TypeAlias = (
    AlloyVoiceInput
    | LanguageInput
    | FormatInput
    | SsmlInput
    | AudioInput
    | VoiceInput
    | DecodeInput
)


class Body(TypedDict):
    input: dict[str, JsonValue]
    voice: dict[str, JsonValue]
    audioConfig: dict[str, JsonValue]


MAPPED = TypeAdapter(Params)
BODY = TypeAdapter(Body)


def fixed_token(
    self: VertexAITextToSpeechConfig,
    credentials: VERTEX_CREDENTIALS_TYPES | None,
    project_id: str | None,
    *,
    _retry_reauth: bool = False,
) -> tuple[str, str]:
    del self, credentials
    return "oracle-token", PROJECT if project_id is None else project_id


def sniff(audio: bytes) -> str | None:
    return TYPE if audio[:1] == PREFIX else None


def mapped(voice: Voice, optional: Params, kwargs: Params) -> Params:
    _, values = CONFIG.map_openai_params("m", optional, voice=voice, kwargs=kwargs)
    return MAPPED.validate_json(json.dumps(values))


def transform(value: str, voice: str | None, optional: Params, params: Params) -> Body:
    data = CONFIG.transform_text_to_speech_request(
        model="m", input=value, voice=voice, optional_params=optional,
        litellm_params=params, headers={},
    )
    return BODY.validate_json(json.dumps(data.get("dict_body")))


def alloy_voice(voices: list[Voice]) -> JsonValue:
    views = [{"voice": v, "dict": mapped(v, {}, {}).get("vertex_voice_dict")} for v in voices]
    return {"voices": views}


def custom_voice_language(voices: list[str]) -> JsonValue:
    return {"voices": [mapped(v, {}, {}).get("vertex_voice_dict") for v in voices]}


def aac_maps_mp3(formats: list[str | None]) -> JsonValue:
    formats_as_params = [{} if f is None else {"response_format": f} for f in formats]
    return {"encodings": [mapped(None, p, {}).get("audioEncoding") for p in formats_as_params]}


def ssml_substring(cases: list[dict[str, JsonValue]]) -> JsonValue:
    views: list[JsonValue] = []
    for case in cases:
        kwargs: Params = {"use_ssml": case.get("use_ssml")}
        try:
            body = transform(str(case.get("input")), None, mapped(None, {}, kwargs), {})
            views.append({"input": body["input"]})
        except ValueError:
            views.append({"error": ERROR})
    return {"inputs": views}


def full_audio_config(cases: list[dict[str, JsonValue]]) -> JsonValue:
    views: list[JsonValue] = []
    for case in cases:
        optional: Params = {}
        if case.get("response_format") is not None:
            optional["response_format"] = case.get("response_format")
        if case.get("speed") is not None:
            optional["speed"] = case.get("speed")
        kwargs: Params = {}
        if case.get("audio_config") is not None:
            kwargs["audioConfig"] = case.get("audio_config")
        views.append(transform("hi", None, mapped(None, optional, kwargs), {})["audioConfig"])
    return {"configs": views}


def voice_dict_precedence(cases: list[dict[str, JsonValue]]) -> JsonValue:
    views: list[JsonValue] = []
    for case in cases:
        voice = case.get("voice")
        optional = mapped(str(voice) if voice is not None else None, {}, {})
        if case.get("mapped_voice") is not None:
            optional["vertex_voice_dict"] = case.get("mapped_voice")
        params: Params = {}
        if case.get("litellm_voice") is not None:
            params["vertex_voice_dict"] = case.get("litellm_voice")
        spoken = None if voice is None else str(voice)
        try:
            views.append(transform("hi", spoken, optional, params)["voice"])
        except ValueError:
            views.append({"error": ERROR})
    return {"voices": views}


def response_view(text: str) -> JsonValue:
    raw = httpx.Response(status_code=STATUS, content=text.encode())
    try:
        reply = CONFIG.transform_text_to_speech_response("m", raw, None)
    except json.JSONDecodeError:
        return {"error": "json"}
    except AttributeError:
        return {"error": "not_object"}
    except TypeError:
        return {"error": "not_text"}
    except ValueError:
        return {"error": ERROR}
    return {
        "status": reply.response.status_code,
        "contentType": reply.response.headers.get("content-type"),
        "bytes": list(reply.content),
    }


def decoded_status_200(bodies: list[str]) -> JsonValue:
    with patch(MEDIA, sniff):
        return {"bodies": [response_view(body) for body in bodies]}


def run(input_value: Input) -> JsonValue:
    result: JsonValue
    with patch.object(VertexAITextToSpeechConfig, "get_access_token", fixed_token):
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
