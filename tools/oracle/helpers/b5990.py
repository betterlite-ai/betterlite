import os
from typing import Protocol, TypeAlias, cast
from unittest.mock import patch

from litellm.llms.huggingface.chat.transformation import HuggingFaceChatConfig
from litellm.types.llms.openai import AllMessageValues

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]
Object: TypeAlias = dict[str, Json]


class ChatConfig(Protocol):
    def validate_environment(
        self,
        headers: dict[str, Json],
        model: str,
        messages: list[AllMessageValues],
        optional_params: dict[str, Json],
        litellm_params: dict[str, Json],
        **credentials: str | None,
    ) -> dict[str, Json]: ...

    def get_complete_url(
        self,
        api_base: str | None,
        api_key: str | None,
        model: str,
        optional_params: dict[str, Json],
        litellm_params: dict[str, Json],
    ) -> str: ...

    def transform_request(
        self,
        model: str,
        messages: list[AllMessageValues],
        optional_params: dict[str, Json],
        litellm_params: dict[str, Json],
        headers: dict[str, Json],
    ) -> dict[str, Json]: ...


def obj(value: Json) -> Object:
    if not isinstance(value, dict):
        raise TypeError("expected an object")
    return value


def obj_or_empty(value: Json | None) -> Object:
    return {} if value is None else obj(value)


def text(fields: Object, name: str) -> str:
    value = fields.get(name)
    if not isinstance(value, str):
        raise TypeError(f"{name} must be a string")
    return value


def optional_text(fields: Object, name: str) -> str | None:
    value = fields.get(name)
    if value is not None and not isinstance(value, str):
        raise TypeError(f"{name} must be a string")
    return value


def messages(fields: Object) -> list[AllMessageValues]:
    value = fields.get("messages", [])
    if not isinstance(value, list):
        raise TypeError("messages must be a list")
    return cast("list[AllMessageValues]", value)


def environment(fields: Object) -> dict[str, str]:
    variables: dict[str, str] = {}
    for name, value in obj_or_empty(fields.get("env")).items():
        if not isinstance(value, str):
            raise TypeError("environment values must be strings")
        variables[name] = value
    return variables


def apply_environment(variables: dict[str, str]) -> None:
    for name in ("HF_API_BASE", "HUGGINGFACE_API_BASE"):
        os.environ.pop(name, None)
    os.environ.update(variables)


def complete_url(config: ChatConfig, step: Object) -> Json:
    apply_environment(environment(step))
    return config.get_complete_url(
        optional_text(step, "api_base"),
        None,
        text(step, "model"),
        {},
        {},
    )


def headers(config: ChatConfig, step: Object) -> Json:
    return config.validate_environment(
        obj_or_empty(step.get("headers")),
        "",
        [],
        {},
        {},
        api_key=optional_text(step, "api_key"),
    )


def request(config: ChatConfig, step: Object) -> Json:
    with patch(
        "litellm.llms.huggingface.chat.transformation._fetch_inference_provider_mapping",
        return_value=step.get("mapping"),
    ):
        return config.transform_request(
            text(step, "model"),
            messages(step),
            obj_or_empty(step.get("optional_params")),
            {"api_base": optional_text(step, "api_base")},
            {},
        )


def execute(config: ChatConfig, step: Object) -> Json:
    kind = text(step, "kind")
    if kind == "complete_url":
        return complete_url(config, step)
    if kind == "validate_environment":
        return headers(config, step)
    if kind == "transform_request":
        return request(config, step)
    raise ValueError(f"unknown step kind {kind}")


def run(value: dict[str, Json]) -> Json:
    config = cast(ChatConfig, HuggingFaceChatConfig())
    steps = value.get("steps", [])
    if not isinstance(steps, list):
        raise TypeError("steps must be a list")
    return [execute(config, obj(step)) for step in steps]
