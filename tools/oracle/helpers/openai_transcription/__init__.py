import json
from typing import Final, Literal, Protocol, TypedDict, cast

import httpx
import litellm
from litellm.utils import extract_duration_from_srt_or_vtt
from pydantic import JsonValue as Json

from .exchange import exchange, response_document
from .probe import PROBE, PROVIDERS, WHISPER, Probe

litellm.suppress_debug_info = True

URL_BASES: Final = [
    None,
    "",
    "https://speech.test/v1",
    "https://speech.test/v1/",
    "https://speech.test/v2",
    "https://speech.test",
]


class ConfigView(Protocol):
    def get_complete_url(self, **kwargs: Json) -> str: ...

    def map_openai_params(self, **kwargs: Json) -> dict[str, Json]: ...

    def validate_environment(self, **kwargs: Json) -> dict[str, Json]: ...


VIEW: Final = cast(ConfigView, WHISPER)  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/openai/transcriptions/whisper_transformation.py#L21-L95


class Input(TypedDict):
    function: Literal["exchange", "params", "duration", "auth", "response"]
    probes: list[dict[str, Json]]


def params(probe: Probe) -> Json:
    fields: dict[str, Json] = {}
    non_default = dict(probe.non_default_params)
    existing = dict(probe.existing_params)
    for label, config in PROVIDERS.items():
        supported = config.get_supported_openai_params(model=probe.model)
        mapped = VIEW.map_openai_params(
            non_default_params=non_default,
            optional_params=existing,
            model=probe.model,
            drop_params=False,
        )
        fields[f"{label}_supported"] = cast(Json, supported)  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/llms/openai.py#L1081
        fields[f"{label}_mapped"] = mapped
    return fields


def durations(probe: Probe) -> Json:
    found: list[Json] = [extract_duration_from_srt_or_vtt(text) for text in probe.texts]
    return {"durations": found}


def auth(probe: Probe) -> Json:
    empty: dict[str, Json] = {}
    urls: list[Json] = [
        VIEW.get_complete_url(
            api_base=base,
            api_key=None,
            model=probe.model,
            optional_params=empty,
            litellm_params=empty,
        )
        for base in URL_BASES
    ]
    headers = VIEW.validate_environment(
        headers=dict(probe.headers),
        model=probe.model,
        messages=[],
        optional_params=empty,
        litellm_params=empty,
        api_key=probe.api_key,
    )
    return {"urls": urls, "headers": headers}


def responses(probe: Probe) -> Json:
    results: list[Json] = []
    for item in probe.bodies:
        reply = httpx.Response(
            200, headers={"content-type": item.content_type}, content=item.body.encode()
        )
        try:
            transformed = WHISPER.transform_audio_transcription_response(reply)
        except json.JSONDecodeError:
            results.append({"ok": False, "kind": "json"})
        except (ValueError, TypeError):
            results.append({"ok": False, "kind": "value"})
        else:
            dumped = cast("dict[str, Json]", transformed.model_dump())  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/utils.py#L2748
            results.append({"ok": True, "response": response_document(dumped)})
    return {"results": results}


def run(input_value: Input) -> Json:
    values: list[Json] = []
    for raw in input_value["probes"]:
        probe = PROBE.validate_python(raw)
        match input_value["function"]:
            case "exchange":
                values.append(exchange(probe))
            case "params":
                values.append(params(probe))
            case "duration":
                values.append(durations(probe))
            case "auth":
                values.append(auth(probe))
            case "response":
                values.append(responses(probe))
    return values
