from collections.abc import Callable, Mapping, MutableMapping, Sequence
from datetime import datetime, tzinfo
from importlib.util import module_from_spec, spec_from_file_location
from pathlib import Path
from typing import TypeAlias, cast
from unittest.mock import patch
from uuid import UUID

import litellm
from litellm.types.utils import ModelResponse

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]

_TRANSFORMATION_PATH = (
    Path(litellm.__file__).parent
    / "a2a_protocol"
    / "litellm_completion_bridge"
    / "transformation.py"
)
_SPEC = spec_from_file_location("b4803_litellm_transformation", _TRANSFORMATION_PATH)
if _SPEC is None or _SPEC.loader is None:
    raise ImportError(f"cannot load LiteLLM transformation at {_TRANSFORMATION_PATH}")
_TRANSFORMATION_MODULE = module_from_spec(_SPEC)
_SPEC.loader.exec_module(_TRANSFORMATION_MODULE)
_MODULE_FIELDS = cast(dict[str, object], vars(_TRANSFORMATION_MODULE))
_TRANSFORMATION = _MODULE_FIELDS["A2ACompletionBridgeTransformation"]
_METHODS = cast(dict[str, object], vars(_TRANSFORMATION))
_CONTEXT = cast(
    Callable[[str, Mapping[str, Json]], object],
    _MODULE_FIELDS["A2AStreamingContext"],
)
_EXTRACT = cast(
    Callable[[Sequence[Json]], str],
    _METHODS["_extract_text_from_a2a_parts"],
)
_APPLY_METADATA = cast(
    Callable[[MutableMapping[str, object], Mapping[str, Json], Mapping[str, Json] | None], None],
    _METHODS["apply_forward_metadata_to_completion_params"],
)
_TO_OPENAI = cast(
    Callable[[Mapping[str, Json]], list[dict[str, object]]],
    _METHODS["a2a_message_to_openai_messages"],
)
_TO_A2A = cast(
    Callable[[ModelResponse, str], dict[str, object]],
    _METHODS["openai_response_to_a2a_response"],
)
_TASK_EVENT = cast(Callable[[object], dict[str, object]], _METHODS["create_task_event"])
_STATUS_EVENT = cast(
    Callable[..., dict[str, object]], _METHODS["create_status_update_event"]
)
_ARTIFACT_EVENT = cast(
    Callable[[object, str], dict[str, object]],
    _METHODS["create_artifact_update_event"],
)


def transform(input_value: dict[str, Json]) -> Json:
    operation = input_value.get("operation")
    request_id = cast(str, input_value.get("request_id", "request"))
    message = cast(Mapping[str, Json], input_value.get("message", {}))
    params = cast(Mapping[str, Json], input_value.get("params", {}))
    if operation == "extract":
        parts = cast(list[Json], input_value.get("parts", []))
        return _EXTRACT(parts)
    if operation == "metadata":
        completion_params = cast(
            MutableMapping[str, object], input_value.get("completion_params", {})
        )
        _APPLY_METADATA(completion_params, message, params)
        return cast(Json, completion_params)
    if operation == "roles":
        return cast(Json, _TO_OPENAI(message))
    if operation == "response":
        response = ModelResponse(**cast(dict[str, object], input_value.get("response", {})))
        return cast(Json, _TO_A2A(response, cast(str, input_value.get("request_id"))))
    return event(operation, _CONTEXT(request_id, message))


def event(operation: Json, context: object) -> Json:
    if operation == "task":
        return cast(Json, _TASK_EVENT(context))
    if operation == "working":
        return cast(Json, _STATUS_EVENT(context, "working", message_text="thinking"))
    if operation == "completed":
        return cast(Json, _STATUS_EVENT(context, "completed", final=True))
    if operation == "artifact":
        return cast(Json, _ARTIFACT_EVENT(context, "answer"))
    raise ValueError(f"unsupported B4803 operation: {operation}")


class FixedTime:
    @staticmethod
    def now(tz: tzinfo | None = None) -> datetime:
        return datetime(1970, 1, 1, tzinfo=tz)


def run(input_value: dict[str, Json]) -> Json:
    ids = (UUID(int=value) for value in range(1, 11))
    with (
        patch.object(_TRANSFORMATION_MODULE, "uuid4", side_effect=ids),
        patch.object(_TRANSFORMATION_MODULE, "datetime", FixedTime),
    ):
        return transform(input_value)
