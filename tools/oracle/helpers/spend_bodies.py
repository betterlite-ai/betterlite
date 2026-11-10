import json
import os
import uuid
from datetime import UTC, datetime, timedelta
from types import SimpleNamespace
from typing import TYPE_CHECKING, TypeAlias, cast
from unittest.mock import patch

import litellm
import litellm._uuid as litellm_uuid
from litellm import constants
from litellm.litellm_core_utils import classifier_logging
from litellm.proxy import proxy_server
from litellm.proxy.spend_tracking import spend_tracking_utils

if TYPE_CHECKING:
    from litellm.types.utils import StandardLoggingPayload, StandardLoggingVectorStoreRequest

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]
EPOCH = datetime(1970, 1, 1, tzinfo=UTC)
MAX_PROMPT_LENGTH_ENV = "MAX_STRING_LENGTH_PROMPT_IN_DB"
STORE_PROMPTS_ENV = "STORE_PROMPTS_IN_SPEND_LOGS"


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


def nullable_string(value: Json, name: str) -> str | None:
    return None if value is None else string_value(value, name)


def configure(*, store_bodies: bool, max_string_length: int, turn_off: bool) -> None:
    proxy_server.general_settings["store_prompts_in_spend_logs"] = store_bodies  # pyright: ignore[reportUnknownMemberType]
    litellm.turn_off_message_logging = turn_off  # pyright: ignore[reportAttributeAccessIssue]
    os.environ[MAX_PROMPT_LENGTH_ENV] = str(max_string_length)


def configure_body(input_value: dict[str, Json]) -> None:
    configure(
        store_bodies=bool_value(input_value["store_bodies"], "store_bodies"),
        max_string_length=int_value(input_value["max_string_length"], "max_string_length"),
        turn_off=bool_value(input_value["turn_off_message_logging"], "turn_off_message_logging"),
    )


def call_kwargs(input_value: dict[str, Json], litellm_params: dict[str, Json]) -> dict[str, object]:
    return {
        "standard_logging_object": input_value.get("standard_logging_object"),
        "realtime_tools": input_value.get("realtime_tools"),
        "standard_callback_dynamic_params": input_value.get("dynamic_params"),
        "litellm_params": litellm_params,
        "realtime_tool_calls": input_value.get("realtime_tool_calls"),
    }


def stored_request(input_value: dict[str, Json], litellm_params: dict[str, Json]) -> str:
    return spend_tracking_utils._get_proxy_server_request_for_spend_logs_payload(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage, reportUnknownMemberType]
        metadata={},
        litellm_params=litellm_params,
        kwargs=call_kwargs(input_value, litellm_params),
    )


def plain(value: object) -> Json:
    return cast(Json, json.loads(json.dumps(value, default=dict)))


def storage_policy(input_value: dict[str, Json]) -> dict[str, Json]:
    config_value = input_value.get("config_value")
    env_value = nullable_string(input_value.get("env_value"), "env_value")
    initial = nullable_string(input_value.get("initial_max_string_length"), "initial")
    runtime_values = [
        nullable_string(item, "runtime_max_string_length")
        for item in list_value(input_value["runtime_max_string_lengths"], "runtime")
    ]
    proxy_server.general_settings["store_prompts_in_spend_logs"] = config_value  # pyright: ignore[reportUnknownMemberType]
    if env_value is None:
        os.environ.pop(STORE_PROMPTS_ENV, None)
    else:
        os.environ[STORE_PROMPTS_ENV] = env_value
    spend_tracking_utils.DEFAULT_MAX_STRING_LENGTH_PROMPT_IN_DB = (  # pyright: ignore[reportAttributeAccessIssue]
        constants.MAX_STRING_LENGTH_PROMPT_IN_DB if initial is None else int(initial)
    )
    store_bodies = spend_tracking_utils.should_store_prompts_and_responses_in_spend_logs()
    lengths: list[Json] = []
    for runtime in runtime_values:
        if runtime is None:
            os.environ.pop(MAX_PROMPT_LENGTH_ENV, None)
        else:
            os.environ[MAX_PROMPT_LENGTH_ENV] = runtime
        lengths.append(spend_tracking_utils._get_max_string_length_prompt_in_db())  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
    return {"store_bodies": store_bodies, "max_string_lengths": lengths}


def body_storage(input_value: dict[str, Json]) -> dict[str, Json]:
    configure_body(input_value)
    litellm_params = object_value(input_value["litellm_params"], "litellm_params")
    response = spend_tracking_utils._get_response_for_spend_logs_payload(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage, reportUnknownMemberType]
        payload=cast("StandardLoggingPayload", {"response": input_value.get("response")}),
        kwargs=call_kwargs(input_value, litellm_params),
    )
    messages = "{}"
    messages_payload = input_value.get("messages_payload")
    if messages_payload is not None:
        messages = spend_tracking_utils._get_messages_for_spend_logs_payload(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage, reportUnknownMemberType]
            standard_logging_payload=cast(
                "StandardLoggingPayload", object_value(messages_payload, "messages_payload")
            )
        )
    return {
        "request": stored_request(input_value, litellm_params),
        "response": response,
        "messages": messages,
    }


def classifier(input_value: dict[str, Json]) -> dict[str, Json]:
    configure_body(input_value)
    litellm_params = object_value(input_value["litellm_params"], "litellm_params")
    standard = input_value.get("standard_logging_object")
    snapshots: list[Json] = []
    for spec in list_value(input_value["snapshots"], "snapshots"):
        spec_object = object_value(spec, "snapshot")
        snapshots.append(
            cast(
                Json,
                classifier_logging.classifier_input_snapshot(
                    spec_object.get("value"),
                    openai_sdk=bool_value(spec_object["openai_sdk"], "openai_sdk"),
                ),
            )
        )
    audit: Json = None
    without: Json = None
    if standard is not None:
        standard_object = object_value(standard, "standard_logging_object")
        audit = cast(Json, classifier_logging.classifier_audit_fields(standard_object))
        without = cast(Json, classifier_logging.without_classifier_audit(standard_object))
    return {
        "snapshots": snapshots,
        "audit": audit,
        "without": without,
        "request": stored_request(input_value, litellm_params),
    }


def vector_and_placeholder(input_value: dict[str, Json]) -> dict[str, Json]:
    configure(
        store_bodies=bool_value(input_value["store_bodies"], "store_bodies"),
        max_string_length=2048,
        turn_off=False,
    )
    litellm_params = object_value(input_value["litellm_params"], "litellm_params")
    vector_store = input_value.get("vector_store")
    requests = (
        None
        if vector_store is None
        else cast(
            "list[StandardLoggingVectorStoreRequest]",
            list_value(vector_store, "vector_store"),
        )
    )
    redacted = spend_tracking_utils._get_vector_store_request_for_spend_logs_payload(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
        requests
    )
    placeholder = spend_tracking_utils._placeholder_stored_request_body(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
        litellm_params,
        string_value(input_value["model_group"], "model_group"),
        string_value(input_value["raw_model"], "raw_model"),
    )
    return {"vector_store": plain(redacted), "placeholder": plain(placeholder)}


def session_value(case: dict[str, Json]) -> str | None:
    metadata = case.get("metadata")
    metadata_object = None if metadata is None else object_value(metadata, "metadata")
    configured_omit = bool_value(case["configured_omit"], "configured_omit")
    proxy_server.general_settings["missing_session_id"] = "omit" if configured_omit else None  # pyright: ignore[reportUnknownMemberType]
    trace_id = case.get("trace_id")
    litellm_trace_id = case.get("litellm_trace_id")
    omit = spend_tracking_utils._omits_session_id_when_missing(metadata_object)  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
    batch = spend_tracking_utils._get_batch_trace_session_id(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
        nullable_string(case.get("call_type"), "call_type"),
        nullable_string(case.get("request_id"), "request_id"),
    )
    kwargs: dict[str, object] = (
        {} if litellm_trace_id is None else {"litellm_trace_id": litellm_trace_id}
    )
    payload = (
        None
        if trace_id is None
        else cast("StandardLoggingPayload", {"trace_id": trace_id})
    )
    fixed = SimpleNamespace(uuid4=lambda: uuid.UUID(string_value(case["injected_uuid"], "uuid")))
    with patch.object(litellm_uuid, "uuid", fixed):
        return spend_tracking_utils._get_session_id_for_spend_log(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
            kwargs=kwargs,
            metadata=metadata_object,
            standard_logging_payload=payload,
            omit_when_missing=omit,
            batch_trace_session_id=batch,
        )


def session(input_value: dict[str, Json]) -> dict[str, Json]:
    cases = list_value(input_value["sessions"], "sessions")
    values: list[Json] = [session_value(object_value(case, "session")) for case in cases]
    durations: list[Json] = []
    for entry in list_value(input_value["durations"], "durations"):
        duration = object_value(entry, "duration")
        durations.append(
            spend_tracking_utils._get_request_duration_ms(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
                datetime.fromisoformat(string_value(duration["start"], "start")),
                datetime.fromisoformat(string_value(duration["end"], "end")),
            )
        )
    timestamps: list[Json] = []
    for item in list_value(input_value["timestamps"], "timestamps"):
        delta = (
            spend_tracking_utils._ensure_datetime_utc(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
                datetime.fromisoformat(string_value(item, "timestamp"))
            )
            - EPOCH
        )
        timestamps.append(delta // timedelta.resolution)
    return {"sessions": values, "durations": durations, "timestamps": timestamps}


def run(input_value: dict[str, Json]) -> Json:
    function = string_value(input_value.get("function"), "function")
    if function == "storage_policy":
        return storage_policy(input_value)
    if function == "body_storage":
        return body_storage(input_value)
    if function == "classifier":
        return classifier(input_value)
    if function == "vector_and_placeholder":
        return vector_and_placeholder(input_value)
    if function == "session":
        return session(input_value)
    raise ValueError(f"unknown spend_bodies function ({litellm.__name__})")
