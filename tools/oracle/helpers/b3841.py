from collections.abc import Mapping
from dataclasses import fields, is_dataclass
from typing import TYPE_CHECKING, TypeAlias, cast

import litellm
from litellm.integrations.otel.model.baggage import promoted_baggage
from litellm.integrations.otel.model.metadata import (
    RequestContext,
    RequestIdentity,
    caller_session_id,
    flatten_metadata,
)
from litellm.integrations.otel.model.trace_controls import caller_trace_controls

if TYPE_CHECKING:
    from litellm.types.utils import StandardLoggingPayload

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]


def run(input_value: dict[str, Json]) -> Json:
    function = input_value.get("function")
    if function == "identity":
        payload = cast("StandardLoggingPayload", input_value["payload"])
        return plain(RequestIdentity.from_payload(payload))
    if function == "context":
        payload = cast("StandardLoggingPayload", input_value["payload"])
        return plain(RequestContext.from_standard_logging_payload(payload))
    if function == "session":
        kwargs = cast("Mapping[str, object]", input_value["kwargs"])
        controls = caller_trace_controls(kwargs)
        return {"session_id": caller_session_id(kwargs, controls)}
    if function == "flatten":
        metadata = cast("Mapping[str, object]", input_value.get("metadata", {}))
        return plain(dict(flatten_metadata(metadata)))
    if function == "baggage":
        identity_values = cast("dict[str, object]", input_value["identity"])
        identity = RequestIdentity(
            call_id=cast("str | None", identity_values.get("call_id")),
            team_id=cast("str | None", identity_values.get("team_id")),
            team_alias=cast("str | None", identity_values.get("team_alias")),
            team_metadata=cast("Mapping[str, object] | None", identity_values.get("team_metadata")),
            key_hash=cast("str | None", identity_values.get("key_hash")),
            end_user=cast("str | None", identity_values.get("end_user")),
            provider_model=cast("str | None", identity_values.get("provider_model")),
            request_route=cast("str | None", identity_values.get("request_route")),
            metadata=cast("Mapping[str, str]", identity_values.get("metadata", {})),
        )
        return plain(promoted_baggage(
            identity,
            cast("str | None", input_value.get("request_model")),
            cast("tuple[str, ...]", input_value.get("promoted_keys", ())),
            cast("tuple[str, ...]", input_value.get("metadata_keys", ())),
            cast("tuple[str, ...]", input_value.get("team_metadata_keys", ())),
        ))
    raise ValueError(f"unknown {litellm.__name__} helper function {function}")


def plain(value: object) -> Json:
    if is_dataclass(value):
        return {
            field.name: plain(cast("object", getattr(value, field.name)))
            for field in fields(value)
        }
    if isinstance(value, Mapping):
        values = cast("Mapping[str, object]", value)
        return {str(key): plain(item) for key, item in values.items()}
    if isinstance(value, tuple):
        items = cast("tuple[object, ...]", value)
        return [plain(item) for item in items]
    if isinstance(value, (str, int, float, bool)) or value is None:
        return value
    return str(value)
