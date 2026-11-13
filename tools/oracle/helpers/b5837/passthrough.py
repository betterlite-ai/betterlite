import os
from collections.abc import Mapping, Sequence
from typing import Protocol, TypedDict, Unpack

import httpx
from litellm.llms.nvidia_nim.passthrough.transformation import NvidiaNimPassthroughConfig
from litellm.types.llms.openai import AllMessageValues
from pydantic import JsonValue, TypeAdapter

from .types import JsonObject, PassthroughCredentials, PassthroughUrl

ADAPTER: TypeAdapter[JsonValue] = TypeAdapter(JsonValue)


class UrlArguments(TypedDict):
    api_base: str | None
    api_key: str | None
    model: str
    endpoint: str
    request_query_params: JsonObject | None
    litellm_params: JsonObject


class EnvironmentArguments(TypedDict):
    headers: Mapping[str, str]
    model: str
    messages: Sequence[AllMessageValues]
    optional_params: Mapping[str, JsonValue]
    litellm_params: Mapping[str, JsonValue]
    api_key: str | None
    api_base: str | None


class PassthroughConfig(Protocol):
    def get_complete_url(self, **arguments: Unpack[UrlArguments]) -> tuple[httpx.URL, str]: ...
    def is_streaming_request(self, endpoint: str, request_data: JsonObject) -> bool: ...
    def validate_environment(self, **arguments: Unpack[EnvironmentArguments]) -> dict[str, str]: ...


PASSTHROUGH: PassthroughConfig = NvidiaNimPassthroughConfig()


def set_env(name: str, value: str | None) -> None:
    if value is None:
        os.environ.pop(name, None)
    else:
        os.environ[name] = value


def passthrough_url(input_value: PassthroughUrl) -> JsonValue:
    urls: list[JsonValue] = []
    for call in input_value["cases"]:
        set_env("NVIDIA_NIM_API_BASE", call["env_base"])
        litellm_params: JsonObject = {}
        if call["model_group"] is not None:
            litellm_params["litellm_metadata"] = {"model_group": call["model_group"]}
        arguments = UrlArguments(
            api_base=call["api_base"],
            api_key=None,
            model=call["model"],
            endpoint=call["endpoint"],
            request_query_params=None,
            litellm_params=litellm_params,
        )
        try:
            url, root = PASSTHROUGH.get_complete_url(**arguments)
        except ValueError:
            urls.append({"ok": False})
            continue
        urls.append({"ok": True, "url": str(url), "root": root})
    set_env("NVIDIA_NIM_API_BASE", None)
    return ADAPTER.validate_python({"urls": urls})


def passthrough_credentials(input_value: PassthroughCredentials) -> JsonValue:
    headers: list[JsonValue] = []
    streaming: list[JsonValue] = []
    for call in input_value["cases"]:
        set_env("NVIDIA_NIM_API_KEY", call["env_key"])
        arguments = EnvironmentArguments(
            headers=dict(call["headers"]),
            model="meta/llama",
            messages=[],
            optional_params={},
            litellm_params={},
            api_key=call["api_key"],
            api_base=None,
        )
        headers.append(PASSTHROUGH.validate_environment(**arguments))
        streaming.append(
            PASSTHROUGH.is_streaming_request("chat/completions", {"stream": call["stream"]})
        )
    set_env("NVIDIA_NIM_API_KEY", None)
    return ADAPTER.validate_python({"headers": headers, "streaming": streaming})


def run_passthrough(input_value: PassthroughUrl | PassthroughCredentials) -> JsonValue:
    match input_value["kind"]:
        case "passthrough_url":
            return passthrough_url(input_value)
        case "passthrough_credentials":
            return passthrough_credentials(input_value)
