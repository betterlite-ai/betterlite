from typing import NotRequired, Protocol, TypeAlias, TypedDict, cast

import httpx
import litellm
from litellm.llms.azure.text_to_speech.transformation import AzureAVATextToSpeechConfig
from pydantic import JsonValue

# an unmapped model prints a provider banner on stdout, which carries the oracle's JSON line
litellm.suppress_debug_info = True

Voice: TypeAlias = str | dict[str, JsonValue] | None
Params: TypeAlias = dict[str, JsonValue]


class Reply(Protocol):
    response: httpx.Response
    content: bytes


class SpeechData(TypedDict):
    ssml_body: str
    headers: dict[str, str]


class Config(Protocol):
    def map_openai_params(
        self, model: str, optional_params: Params, voice: Voice = None, *, kwargs: Params
    ) -> tuple[str | None, Params]: ...

    def validate_environment(
        self, headers: dict[str, str], model: str, api_key: str | None = None
    ) -> dict[str, JsonValue]: ...

    def get_complete_url(self, model: str, api_base: str | None, litellm_params: Params) -> str: ...

    def transform_text_to_speech_request(  # ast-grep-ignore: no-long-names
        self, **kwargs: JsonValue
    ) -> SpeechData: ...

    def transform_text_to_speech_response(  # ast-grep-ignore: no-long-names
        self, model: str, raw_response: httpx.Response, logging_obj: JsonValue
    ) -> Reply: ...


CONFIG = cast(Config, AzureAVATextToSpeechConfig())  # type: ignore[no-untyped-call]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/azure/text_to_speech/transformation.py#L145-L495


class Texts(TypedDict):
    inputs: list[str]


class Speeds(TypedDict):
    speeds: list[float | None]


class VoiceCase(TypedDict, total=False):
    voice: Voice
    lang: str


class VoiceCases(TypedDict):
    cases: list[VoiceCase]


class StyleCase(TypedDict, total=False):
    style: str
    styledegree: str
    role: str
    lang: str


class StyleCases(TypedDict):
    cases: list[StyleCase]


class Bases(TypedDict):
    api_bases: list[str | None]


class Formats(TypedDict):
    formats: list[str | None]


class BinaryCase(TypedDict):
    status: int
    body: str
    content_type: NotRequired[str]


class BinaryCases(TypedDict):
    cases: list[BinaryCase]


class Voices(TypedDict):
    voices: list[Voice]


class HeaderCase(TypedDict):
    headers: dict[str, str]
    api_key: NotRequired[str]


class HeaderCases(TypedDict):
    cases: list[HeaderCase]


class Input(TypedDict, total=False):
    plain_text_ssml_escaping: Texts
    speed_rate_boundaries: Speeds
    voice_language: VoiceCases
    style_role_element: StyleCases
    existing_ssml: Texts
    cognitive_to_tts_endpoint: Bases
    response_format_header: Formats
    binary_native_speech: BinaryCases
    native_voice_map: Voices
    native_speech_headers: HeaderCases


def request_view(
    text: str, voice: Voice, optional: Params, kwargs: Params
) -> tuple[str | None, Params, SpeechData]:
    name, params = CONFIG.map_openai_params("m", optional, voice=voice, kwargs=kwargs)
    data = CONFIG.transform_text_to_speech_request(
        model="m",
        input=text,
        voice=name,
        optional_params=params,
        litellm_params={},
        headers={},
    )
    return name, params, data


def text_views(payload: Texts) -> JsonValue:
    bodies: list[JsonValue] = []
    for text in payload["inputs"]:
        _, _, data = request_view(text, None, {}, {})
        bodies.append(data["ssml_body"])
    return {"bodies": bodies}


def speed_views(payload: Speeds) -> JsonValue:
    speeds: list[JsonValue] = []
    for speed in payload["speeds"]:
        optional: Params = {} if speed is None else {"speed": speed}
        _, params, data = request_view("hi", None, optional, {})
        speeds.append({"rate": params.get("rate"), "ssml": data["ssml_body"]})
    return {"speeds": speeds}


def voice_entry(text: str, voice: Voice, lang: str | None) -> JsonValue:
    kwargs: Params = {} if lang is None else {"lang": lang}
    name, _, data = request_view(text, voice, {}, kwargs)
    return {"voice": name, "ssml": data["ssml_body"]}


def voice_language_views(payload: VoiceCases) -> JsonValue:
    voices = [voice_entry("hi", case.get("voice"), case.get("lang")) for case in payload["cases"]]
    return {"voices": voices}


def style_views(payload: StyleCases) -> JsonValue:
    cases: list[JsonValue] = []
    for case in payload["cases"]:
        kwargs: Params = {}
        style = case.get("style")
        if style is not None:
            kwargs["style"] = style
        styledegree = case.get("styledegree")
        if styledegree is not None:
            kwargs["styledegree"] = styledegree
        role = case.get("role")
        if role is not None:
            kwargs["role"] = role
        lang = case.get("lang")
        if lang is not None:
            kwargs["lang"] = lang
        _, params, data = request_view("hi", None, {}, kwargs)
        cases.append({
            "style": params.get("style"),
            "styledegree": params.get("styledegree"),
            "role": params.get("role"),
            "lang": params.get("lang"),
            "ssml": data["ssml_body"],
        })
    return {"cases": cases}


def ssml_views(payload: Texts) -> JsonValue:
    inputs: list[JsonValue] = []
    for text in payload["inputs"]:
        _, _, data = request_view(text, None, {}, {})
        inputs.append({
            "ssml": data["ssml_body"],
            "header": data["headers"].get("X-Microsoft-OutputFormat"),
        })
    return {"inputs": inputs}


def endpoint_value(api_base: str | None) -> JsonValue:
    try:
        return {"url": CONFIG.get_complete_url("m", api_base, {})}
    except ValueError:
        return {"error": True}


def endpoint_views(payload: Bases) -> JsonValue:
    results: list[JsonValue] = [endpoint_value(api_base) for api_base in payload["api_bases"]]
    return {"results": results}


def format_views(payload: Formats) -> JsonValue:
    formats: list[JsonValue] = []
    for name in payload["formats"]:
        optional: Params = {} if name is None else {"response_format": name}
        _, params, data = request_view("hi", None, optional, {})
        formats.append({
            "output_format": params.get("output_format"),
            "header": data["headers"].get("X-Microsoft-OutputFormat"),
        })
    return {"formats": formats}


def binary_views(payload: BinaryCases) -> JsonValue:
    cases: list[JsonValue] = []
    for case in payload["cases"]:
        headers: dict[str, str] = {}
        content_type = case.get("content_type")
        if content_type is not None:
            headers["content-type"] = content_type
        raw = httpx.Response(
            status_code=case["status"], content=case["body"].encode(), headers=headers
        )
        reply = CONFIG.transform_text_to_speech_response("m", raw, None)
        content_types = reply.response.headers.get_list("content-type")
        cases.append({
            "status": reply.response.status_code,
            "contentType": next(iter(content_types), None),
            "bytes": list(reply.content),
        })
    return {"cases": cases}


def voice_map_views(payload: Voices) -> JsonValue:
    voices = [voice_entry("hi", voice, None) for voice in payload["voices"]]
    return {"voices": voices}


def header_views(payload: HeaderCases) -> JsonValue:
    cases: list[JsonValue] = []
    for case in payload["cases"]:
        api_key = case.get("api_key")
        validated = CONFIG.validate_environment(dict(case["headers"]), "m", api_key=api_key)
        cases.append({"headers": validated})
    return {"cases": cases}


def transport_views(input_value: Input) -> JsonValue:
    if "cognitive_to_tts_endpoint" in input_value:
        return endpoint_views(input_value["cognitive_to_tts_endpoint"])
    if "response_format_header" in input_value:
        return format_views(input_value["response_format_header"])
    if "binary_native_speech" in input_value:
        return binary_views(input_value["binary_native_speech"])
    if "native_voice_map" in input_value:
        return voice_map_views(input_value["native_voice_map"])
    if "native_speech_headers" in input_value:
        return header_views(input_value["native_speech_headers"])
    raise ValueError("expected a transport variant")


def run(input_value: Input) -> JsonValue:
    if "plain_text_ssml_escaping" in input_value:
        return text_views(input_value["plain_text_ssml_escaping"])
    if "speed_rate_boundaries" in input_value:
        return speed_views(input_value["speed_rate_boundaries"])
    if "voice_language" in input_value:
        return voice_language_views(input_value["voice_language"])
    if "style_role_element" in input_value:
        return style_views(input_value["style_role_element"])
    if "existing_ssml" in input_value:
        return ssml_views(input_value["existing_ssml"])
    return transport_views(input_value)
