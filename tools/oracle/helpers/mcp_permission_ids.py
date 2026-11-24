from copy import deepcopy
from typing import Literal, TypeAlias, TypedDict, cast

from litellm.proxy._types import ObjectPermissionDict
from litellm.proxy.management_helpers.object_permission_utils import (
    _dedupe_preserving_order,  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/management_helpers/object_permission_utils.py#L240-L248
    _drop_stale_object_permission_mcp_identifiers,  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/management_helpers/object_permission_utils.py#L408-L422
    _flatten_resolved_mcp_server_ids,  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/management_helpers/object_permission_utils.py#L425-L428
)
from pydantic import JsonValue


class DedupeCall(TypedDict):
    function: Literal["dedupe"]
    values: list[str]


class CleanupCall(TypedDict):
    function: Literal["cleanup"]
    object_permission: JsonValue
    identifiers: dict[str, list[str]]


class FlattenCall(TypedDict):
    function: Literal["flatten"]
    identifiers: dict[str, list[str]]


Call: TypeAlias = DedupeCall | CleanupCall | FlattenCall


def run(input_value: Call) -> JsonValue | list[str]:
    match input_value["function"]:
        case "dedupe":
            return _dedupe_preserving_order(input_value["values"])
        case "cleanup":
            return cleanup_answer(input_value)
        case "flatten":
            return flatten_answer(input_value)


def cleanup_answer(call: CleanupCall) -> JsonValue:
    permission = deepcopy(call["object_permission"])
    identifiers = {
        identifier: set(ids) for identifier, ids in call["identifiers"].items()
    }
    declared = cast(ObjectPermissionDict, permission)  # the case supplies raw stored JSON: https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/management_helpers/object_permission_utils.py#L408-L422
    _drop_stale_object_permission_mcp_identifiers(  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/management_helpers/object_permission_utils.py#L408-L422
        declared, identifiers
    )
    return permission


def flatten_answer(call: FlattenCall) -> list[str]:
    identifiers = {
        identifier: set(ids) for identifier, ids in call["identifiers"].items()
    }
    return sorted(_flatten_resolved_mcp_server_ids(identifiers))  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/management_helpers/object_permission_utils.py#L425-L428
