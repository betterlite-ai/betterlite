import os
from typing import TYPE_CHECKING, TypeAlias, cast

import litellm
from litellm.proxy import proxy_server
from litellm.proxy.spend_tracking import spend_tracking_utils
from pydantic import BaseModel, ConfigDict

if TYPE_CHECKING:
    from litellm.types.utils import (
        StandardLoggingGuardrailInformation,
        StandardLoggingPayloadErrorInformation,
    )

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]


class FixtureObject:
    """A foreign object whose attributes live in its __dict__."""


class FixtureModel(BaseModel):
    model_config = ConfigDict(extra="allow")


def object_value(value: Json, name: str) -> dict[str, Json]:
    if not isinstance(value, dict):
        raise ValueError(f"{name} must be an object")
    return value


def list_value(value: Json, name: str) -> list[Json]:
    if not isinstance(value, list):
        raise ValueError(f"{name} must be a list")
    return value


def string_value(value: Json, name: str) -> str:
    if not isinstance(value, str):
        raise ValueError(f"{name} must be a string")
    return value


def int_value(value: Json, name: str) -> int:
    if not isinstance(value, int) or isinstance(value, bool):
        raise ValueError(f"{name} must be an integer")
    return value


def bool_value(value: Json, name: str) -> bool:
    if not isinstance(value, bool):
        raise ValueError(f"{name} must be a boolean")
    return value


def decode(value: Json, named: dict[int, object]) -> object:
    if isinstance(value, dict):
        return decode_object(value, named)
    if isinstance(value, list):
        return [decode(item, named) for item in value]
    return value


def decode_object(value: dict[str, Json], named: dict[int, object]) -> object:
    kind = value.get("$py")
    if kind is None:
        return {key: decode(item, named) for key, item in value.items()}
    if kind == "ref":
        return named[int_value(value.get("id"), "id")]
    if kind == "mapping":
        return decode_mapping(value, named)
    if kind == "list":
        return decode_sequence(value, named)
    if kind == "model":
        return decode_model(value, named)
    if kind == "object":
        return decode_foreign_object(value, named)
    raise ValueError("unknown $py node kind")


def decode_mapping(value: dict[str, Json], named: dict[int, object]) -> dict[str, object]:
    node_id = int_value(value.get("id"), "id")
    entries = object_value(value.get("entries"), "entries")
    created: dict[str, object] = {}
    named[node_id] = created
    created.update({key: decode(item, named) for key, item in entries.items()})
    return created


def decode_sequence(value: dict[str, Json], named: dict[int, object]) -> list[object]:
    node_id = int_value(value.get("id"), "id")
    items = list_value(value.get("items"), "items")
    created: list[object] = []
    named[node_id] = created
    created.extend(decode(item, named) for item in items)
    return created


def decode_model(value: dict[str, Json], named: dict[int, object]) -> object:
    node_id = int_value(value.get("id"), "id")
    entries = object_value(value.get("entries"), "entries")
    created = FixtureModel(**{key: decode(item, named) for key, item in entries.items()})
    named[node_id] = created
    return created


def decode_foreign_object(value: dict[str, Json], named: dict[int, object]) -> object:
    node_id = int_value(value.get("id"), "id")
    entries = object_value(value.get("entries"), "entries")
    created = FixtureObject()
    named[node_id] = created
    vars(created).update({key: decode(item, named) for key, item in entries.items()})
    return created


def sanitize(body: object, max_string_length: int) -> object:
    return spend_tracking_utils._sanitize_request_body_for_spend_logs_payload(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage, reportUnknownMemberType, reportUnknownVariableType]
        cast(dict[str, object], body), max_string_length_prompt_in_db=max_string_length
    )


def sanitize_body(spec: Json) -> Json:
    body_spec = object_value(spec, "body spec")
    body = object_value(body_spec.get("body"), "body")
    limit = int_value(body_spec.get("max_string_length"), "max_string_length")
    return {"sanitized": cast(Json, sanitize(decode(body, {}), limit))}


def error_information(input_value: dict[str, Json]) -> Json:
    information = object_value(input_value.get("information"), "information")
    store_bodies = bool_value(input_value.get("store_bodies"), "store_bodies")
    limit = int_value(input_value.get("max_string_length"), "max_string_length")
    os.environ["MAX_STRING_LENGTH_PROMPT_IN_DB"] = str(limit)
    proxy_server.general_settings["store_prompts_in_spend_logs"] = store_bodies  # pyright: ignore[reportUnknownMemberType]
    decoded = decode(information, {})
    return cast(
        Json,
        spend_tracking_utils._sanitize_error_information_for_spend_logs(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
            cast("StandardLoggingPayloadErrorInformation", decoded), None
        ),
    )


def guardrail_information(input_value: dict[str, Json]) -> Json:
    information = object_value(input_value.get("information"), "information")
    store_bodies = bool_value(input_value.get("store_bodies"), "store_bodies")
    proxy_server.general_settings["store_prompts_in_spend_logs"] = store_bodies  # pyright: ignore[reportUnknownMemberType]
    decoded = decode(information, {})
    return cast(
        Json,
        spend_tracking_utils._sanitize_guardrail_information_for_spend_logs(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
            cast("list[StandardLoggingGuardrailInformation] | None", decoded)
        ),
    )


def raw_model(input_value: dict[str, Json]) -> Json:
    information = object_value(input_value.get("information"), "information")
    model = string_value(input_value.get("raw_model"), "raw_model")
    decoded = decode(information, {})
    return cast(
        Json,
        spend_tracking_utils._scrub_raw_model_from_error_information(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
            cast("StandardLoggingPayloadErrorInformation", decoded), model
        ),
    )


def sanitize_and_project(input_value: dict[str, Json]) -> Json:
    limit = int_value(input_value.get("max_string_length"), "max_string_length")
    sanitize_body_spec = object_value(input_value.get("sanitize_body"), "sanitize_body")
    project_body_spec = object_value(input_value.get("project_body"), "project_body")
    sanitized = sanitize(decode(sanitize_body_spec, {}), limit)
    projected = spend_tracking_utils._convert_to_json_serializable_dict(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
        decode(project_body_spec, {})
    )
    return {"sanitized": cast(Json, sanitized), "projected": cast(Json, projected)}


def run(input_value: dict[str, Json]) -> Json:
    operation = string_value(input_value.get("operation"), "operation")
    if operation == "sanitize_request_body":
        return {
            "result": [
                sanitize_body(spec) for spec in list_value(input_value.get("bodies"), "bodies")
            ]
        }
    if operation == "sanitize_error_information":
        return {"result": error_information(input_value)}
    if operation == "sanitize_guardrail_information":
        return {"result": guardrail_information(input_value)}
    if operation == "scrub_raw_model":
        return {"result": raw_model(input_value)}
    if operation == "sanitize_and_project":
        return {"result": sanitize_and_project(input_value)}
    raise ValueError(f"unknown spend_sanitize operation ({litellm.__name__})")
