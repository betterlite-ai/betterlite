from typing import TypeAlias, cast

from litellm.caching._embedding_router import (
    build_router_embedding_metadata,
    resolve_embedding_max_input_tokens,
    resolve_embedding_router,
    resolve_embedding_timeout,
    truncate_embedding_input,
)
from litellm.router import Router

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]


def object_value(value: Json, name: str) -> dict[str, Json]:
    if not isinstance(value, dict):
        raise ValueError(f"{name} must be an object")
    return cast(dict[str, Json], value)


def array_value(value: Json, name: str) -> list[Json]:
    if not isinstance(value, list):
        raise ValueError(f"{name} must be a list")
    return cast(list[Json], value)


def string_value(value: Json, name: str) -> str:
    if not isinstance(value, str):
        raise ValueError(f"{name} must be a string")
    return value


def integer_value(value: Json, name: str) -> int:
    if not isinstance(value, int) or isinstance(value, bool):
        raise ValueError(f"{name} must be an integer")
    return value


def optional_integer(value: Json, name: str) -> int | None:
    if value is None:
        return None
    return integer_value(value, name)


def optional_number(value: Json, name: str) -> float | None:
    if value is None:
        return None
    if not isinstance(value, float):
        raise ValueError(f"{name} must be a number")
    return value


def boolean_value(value: Json, name: str) -> bool:
    if not isinstance(value, bool):
        raise ValueError(f"{name} must be a boolean")
    return value


def model_rows(input_value: dict[str, Json]) -> list[dict[str, Json]] | None:
    value = input_value.get("model_list")
    if value is None:
        return None
    return [object_value(row, "model_list entry") for row in array_value(value, "model_list")]


def deployments(rows: list[dict[str, Json]] | None) -> list[dict[str, object]]:
    built: list[dict[str, object]] = []
    for row in rows or []:
        name = row.get("model_name")
        if not isinstance(name, str):
            continue
        deployment: dict[str, object] = {
            "model_name": name,
            "litellm_params": {"model": f"openai/{name}", "api_key": "sk-placeholder"},
        }
        model_info = row.get("model_info")
        if isinstance(model_info, dict):
            deployment["model_info"] = model_info
        built.append(deployment)
    return built


def router(input_value: dict[str, Json], rows: list[dict[str, Json]] | None) -> Router | None:
    if not boolean_value(input_value.get("router_present"), "router_present"):
        return None
    return Router(model_list=deployments(rows))


def router_resolution(input_value: dict[str, Json]) -> Json:
    embedding_model = string_value(input_value.get("embedding_model"), "embedding_model")
    rows = model_rows(input_value)
    resolved = resolve_embedding_router(embedding_model, router(input_value, rows), rows)
    return {"resolved": resolved is not None}


def metadata(input_value: dict[str, Json]) -> Json:
    caller = input_value.get("metadata")
    request_metadata = None if caller is None else object_value(caller, "metadata")
    return {"metadata": cast(Json, build_router_embedding_metadata(request_metadata))}


def max_input_tokens(input_value: dict[str, Json]) -> Json:
    configured = optional_integer(input_value.get("configured"), "configured")
    embedding_model = string_value(input_value.get("embedding_model"), "embedding_model")
    rows = model_rows(input_value)
    resolved = resolve_embedding_max_input_tokens(
        configured, embedding_model, router(input_value, rows)
    )
    return {"max_input_tokens": resolved}


def timeout(input_value: dict[str, Json]) -> Json:
    configured = optional_number(input_value.get("configured"), "configured")
    return {"timeout": resolve_embedding_timeout(configured)}


def truncate(input_value: dict[str, Json]) -> Json:
    prompt = string_value(input_value.get("prompt"), "prompt")
    embedding_model = string_value(input_value.get("embedding_model"), "embedding_model")
    limit = optional_integer(input_value.get("max_input_tokens"), "max_input_tokens")
    return {"prompt": truncate_embedding_input(prompt, embedding_model, limit)}


def run(input_value: dict[str, Json]) -> Json:
    operation = string_value(input_value.get("operation"), "operation")
    match operation:
        case "router":
            return router_resolution(input_value)
        case "metadata":
            return metadata(input_value)
        case "max_input_tokens":
            return max_input_tokens(input_value)
        case "timeout":
            return timeout(input_value)
        case "truncate":
            return truncate(input_value)
        case _:
            raise ValueError(f"unsupported operation {operation}")
