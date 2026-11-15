from typing import Literal, TypedDict

from litellm.llms.nvidia_nim.passthrough.transformation import NvidiaNimPassthroughConfig
from pydantic import JsonValue

from .config import JSON, JsonObject, PassthroughConfig, set_env

PASSTHROUGH: PassthroughConfig = NvidiaNimPassthroughConfig()


class PassthroughUrlCase(TypedDict):
    api_base: str | None
    env_base: str | None
    model: str
    endpoint: str
    model_group: str | None


class PassthroughUrlInput(TypedDict):
    kind: Literal["passthrough_url"]
    cases: list[PassthroughUrlCase]


class PassthroughCredentialsCase(TypedDict):
    api_key: str | None
    env_key: str | None
    headers: dict[str, str]
    stream: JsonValue


class PassthroughCredentialsInput(TypedDict):
    kind: Literal["passthrough_credentials"]
    cases: list[PassthroughCredentialsCase]


def passthrough_url(input_value: PassthroughUrlInput) -> JsonValue:
    urls: list[JsonValue] = []
    for case in input_value["cases"]:
        set_env("NVIDIA_NIM_API_BASE", case["env_base"])
        litellm_params: JsonObject = {}
        if case["model_group"] is not None:
            litellm_params["litellm_metadata"] = {"model_group": case["model_group"]}
        try:
            url, root = PASSTHROUGH.get_complete_url(
                api_base=case["api_base"],
                api_key=None,
                model=case["model"],
                endpoint=case["endpoint"],
                request_query_params=None,
                litellm_params=litellm_params,
            )
        except ValueError:
            urls.append({"ok": False})
            continue
        urls.append({"ok": True, "url": str(url), "root": root})
    set_env("NVIDIA_NIM_API_BASE", None)
    return {"urls": urls}


def passthrough_credentials(input_value: PassthroughCredentialsInput) -> JsonValue:
    headers = [
        JSON.validate_python(
            PASSTHROUGH.validate_environment(
                headers=dict(case["headers"]),
                model="meta/llama",
                messages=[],
                optional_params={},
                litellm_params={},
                api_key=case["api_key"],
            )
        )
        for case in input_value["cases"]
    ]
    streaming: list[JsonValue] = [
        PASSTHROUGH.is_streaming_request("chat/completions", {"stream": case["stream"]})
        for case in input_value["cases"]
    ]
    return {"headers": headers, "streaming": streaming}
