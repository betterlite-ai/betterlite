import json
from collections.abc import Sequence
from typing import Final, Literal, Protocol, TypedDict

import httpx
import litellm
from litellm.types.llms.openai import AllMessageValues
from litellm.utils import extract_duration_from_srt_or_vtt
from pydantic import JsonValue as Json

from .exchange import exchange, response_document
from .probe import PROBE, PROVIDERS, Probe

litellm.suppress_debug_info = True

URL_BASES: Final = [
    None,
    "",
    "https://speech.test/v1",
    "https://speech.test/v1/",
    "https://speech.test/v2",
    "https://speech.test",
]


class NoMessages(list[AllMessageValues]):
    pass


class ConfigView(Protocol):
    def get_complete_url(
        self,
        *,
        api_base: str | None,
        api_key: str | None,
        model: str,
        optional_params: dict[str, Json],
        litellm_params: dict[str, Json],
    ) -> str: ...

    def map_openai_params(
        self,
        *,
        non_default_params: dict[str, Json],
        optional_params: dict[str, Json],
        model: str,
        drop_params: bool,
    ) -> dict[str, Json]: ...

    def validate_environment(
        self,
        _headers: dict[str, Json],
        _model: str,
        _messages: list[AllMessageValues],
        _optional_params: dict[str, Json],
        _litellm_params: dict[str, Json],
        _api_key: str | None,
        /,
    ) -> dict[str, Json]: ...

    def get_supported_openai_params(self, model: str) -> Sequence[str]: ...


class Input(TypedDict):
    function: Literal["exchange", "params", "duration", "auth", "response"]
    probes: list[dict[str, Json]]


def mapped(config: ConfigView, probe: Probe) -> tuple[list[Json], dict[str, Json]]:
    non_default = dict(probe.non_default_params)
    existing = dict(probe.existing_params)
    supported: list[Json] = list(config.get_supported_openai_params(model=probe.model))
    converted = config.map_openai_params(
        non_default_params=non_default,
        optional_params=existing,
        model=probe.model,
        drop_params=False,
    )
    return supported, converted


def params(probe: Probe) -> Json:
    fields: dict[str, Json] = {}
    for label, config in PROVIDERS.items():
        supported, converted = mapped(config, probe)
        fields[f"{label}_supported"] = supported
        fields[f"{label}_mapped"] = converted
    return fields


def durations(probe: Probe) -> Json:
    found: list[Json] = [extract_duration_from_srt_or_vtt(text) for text in probe.texts]
    return {"durations": found}


def auth(whisper: ConfigView, probe: Probe) -> Json:
    empty: dict[str, Json] = {}
    urls: list[Json] = [
        whisper.get_complete_url(
            api_base=base,
            api_key=None,
            model=probe.model,
            optional_params=empty,
            litellm_params=empty,
        )
        for base in URL_BASES
    ]
    headers = whisper.validate_environment(
        dict(probe.headers), probe.model, NoMessages(), empty, empty, probe.api_key
    )
    return {"urls": urls, "headers": headers}


def responses(probe: Probe) -> Json:
    results: list[Json] = []
    for item in probe.bodies:
        reply = httpx.Response(
            200, headers={"content-type": item.content_type}, content=item.body.encode()
        )
        try:
            transformed = PROVIDERS["whisper"].transform_audio_transcription_response(reply)
        except json.JSONDecodeError:
            results.append({"ok": False, "kind": "json"})
        except (ValueError, TypeError):
            results.append({"ok": False, "kind": "value"})
        else:
            results.append({"ok": True, "response": response_document(transformed)})
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
                values.append(auth(PROVIDERS["whisper"], probe))
            case "response":
                values.append(responses(probe))
    return values
