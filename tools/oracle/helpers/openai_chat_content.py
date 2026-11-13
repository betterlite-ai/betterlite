import asyncio
import os
from typing import TypeAlias, cast

import litellm
from litellm.exceptions import BadRequestError
from litellm.llms.openai.chat.gpt_transformation import OpenAIGPTConfig
from litellm.types.llms.openai import AllMessageValues

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]
Object: TypeAlias = dict[str, Json]


class _BaseClassOpenAIGPTConfig(OpenAIGPTConfig):
    _is_base_class = True

    def __init__(self) -> None:
        pass


def obj(value: Json) -> Object:
    if not isinstance(value, dict):
        raise TypeError("expected object")
    return value


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


def flag(fields: Object, name: str) -> bool:
    value = fields.get(name, False)
    if not isinstance(value, bool):
        raise TypeError(f"{name} must be a boolean")
    return value


def environment(fields: Object) -> dict[str, str]:
    raw = obj(fields.get("environment", {}))
    variables = {name: value for name, value in raw.items() if isinstance(value, str)}
    if len(variables) != len(raw):
        raise TypeError("environment values must be strings")
    return variables


def messages(fields: Object) -> list[AllMessageValues]:
    value = fields.get("messages", [])
    if not isinstance(value, list):
        raise TypeError("messages must be a list")
    return cast("list[AllMessageValues]", value)


def optional_params(fields: Object) -> dict[str, object]:
    return cast("dict[str, object]", obj(fields.get("optional_params", {})))


def apply_environment(fields: Object) -> None:
    variables = environment(fields)
    for name in ("OPENAI_BASE_URL", "OPENAI_API_BASE"):
        os.environ.pop(name, None)
    os.environ.update(variables)


def transform(fields: Object) -> Json:
    mode = fields.get("mode", "sync")
    litellm.openai_system_messages_first = flag(fields, "system_messages_first")
    litellm.api_base = optional_text(fields, "global_api_base")
    apply_environment(fields)
    model = text(fields, "model")
    litellm_params: dict[str, object] = {
        "custom_llm_provider": optional_text(fields, "provider"),
        "api_base": optional_text(fields, "api_base"),
    }
    if mode == "sync":
        request = cast(
            Json,
            OpenAIGPTConfig().transform_request(  # pyright: ignore[reportUnknownMemberType]
                model=model,
                messages=messages(fields),
                optional_params=optional_params(fields),
                litellm_params=litellm_params,
                headers={},
            ),
        )
    elif mode == "async_base_class":
        request = cast(
            Json,
            asyncio.run(
                _BaseClassOpenAIGPTConfig().async_transform_request(  # pyright: ignore[reportUnknownMemberType, reportUnknownArgumentType]
                    model=model,
                    messages=messages(fields),
                    optional_params=optional_params(fields),
                    litellm_params=litellm_params,
                    headers={},
                ),
            ),
        )
    elif mode == "async_subclass":
        request = cast(
            Json,
            asyncio.run(
                OpenAIGPTConfig().async_transform_request(  # pyright: ignore[reportUnknownMemberType, reportUnknownArgumentType]
                    model=model,
                    messages=messages(fields),
                    optional_params=optional_params(fields),
                    litellm_params=litellm_params,
                    headers={},
                ),
            ),
        )
    else:
        raise ValueError(f"unknown transform mode: {mode}")
    return request


def outcome(fields: Object) -> Json:
    try:
        return {"ok": True, "value": transform(fields)}
    except BadRequestError as error:
        return {"ok": False, "status": error.status_code}


def run(argument: Json) -> Json:
    fields = obj(argument)
    if len(fields) != 1:
        raise ValueError("expected one openai_chat_content variant")
    name, value = next(iter(fields.items()))
    if name != "transform_requests":
        raise ValueError(f"unknown openai_chat_content variant: {name}")
    requests = obj(value).get("requests", [])
    if not isinstance(requests, list):
        raise TypeError("requests must be a list")
    return [outcome(obj(request)) for request in requests]
