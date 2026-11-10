import json
import os
import uuid
from datetime import UTC, datetime
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
MICROS_PER_DAY = 86_400_000_000
MICROS_PER_SECOND = 1_000_000
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
    if value is None:
        return None
    return string_value(value, name)


def optional_object(input_value: dict[str, Json], name: str) -> dict[str, Json] | None:
    value = input_value.get(name)
    return None if value is None else object_value(value, name)


def optional_string(input_value: dict[str, Json], name: str) -> str | None:
    return nullable_string(input_value.get(name), name)


def configure(
    *, store_bodies: bool, max_string_length: int, turn_off_message_logging: bool
) -> None:
    proxy_server.general_settings["store_prompts_in_spend_logs"] = store_bodies  # pyright: ignore[reportUnknownMemberType]
    litellm.turn_off_message_logging = turn_off_message_logging  # pyright: ignore[reportAttributeAccessIssue]
    os.environ[MAX_PROMPT_LENGTH_ENV] = str(max_string_length)


def call_kwargs(input_value: dict[str, Json], litellm_params: dict[str, Json]) -> dict[str, object]:
    return {
        "standard_logging_object": input_value.get("standard_logging_object"),
        "realtime_tools": input_value.get("realtime_tools"),
        "standard_callback_dynamic_params": input_value.get("dynamic_params"),
        "litellm_params": litellm_params,
        "realtime_tool_calls": input_value.get("realtime_tool_calls"),
    }


def stored_request(input_value: dict[str, Json], litellm_params: dict[str, Json]) -> Json:
    return cast(
        Json,
        spend_tracking_utils._get_proxy_server_request_for_spend_logs_payload(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage, reportUnknownMemberType]
            metadata={},
            litellm_params=litellm_params,
            kwargs=call_kwargs(input_value, litellm_params),
        ),
    )


def plain(value: object) -> Json:
    return cast(Json, json.loads(json.dumps(value, default=dict)))


def epoch_micros(moment: datetime) -> int:
    delta = moment - EPOCH
    return delta.days * MICROS_PER_DAY + delta.seconds * MICROS_PER_SECOND + delta.microseconds


def storage_policy(input_value: dict[str, Json]) -> Json:
    config_value = input_value.get("config_value")
    env_value = optional_string(input_value, "env_value")
    initial = optional_string(input_value, "initial_max_string_length")
    runtime_entries = list_value(
        input_value.get("runtime_max_string_lengths"), "runtime_max_string_lengths"
    )
    runtime_values = [
        nullable_string(item, "runtime_max_string_length") for item in runtime_entries
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
    lengths: list[int] = []
    for runtime in runtime_values:
        if runtime is None:
            os.environ.pop(MAX_PROMPT_LENGTH_ENV, None)
        else:
            os.environ[MAX_PROMPT_LENGTH_ENV] = runtime
        lengths.append(spend_tracking_utils._get_max_string_length_prompt_in_db())  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
    return cast(Json, {"store_bodies": store_bodies, "max_string_lengths": lengths})


def body_storage(input_value: dict[str, Json]) -> Json:
    configure(
        store_bodies=bool_value(input_value.get("store_bodies"), "store_bodies"),
        max_string_length=int_value(input_value.get("max_string_length"), "max_string_length"),
        turn_off_message_logging=bool_value(
            input_value.get("turn_off_message_logging"), "turn_off_message_logging"
        ),
    )
    litellm_params = object_value(input_value.get("litellm_params"), "litellm_params")
    response = spend_tracking_utils._get_response_for_spend_logs_payload(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage, reportUnknownMemberType]
        payload=cast(
            "StandardLoggingPayload", {"response": input_value.get("response")}
        ),
        kwargs=call_kwargs(input_value, litellm_params),
    )
    messages = "{}"
    messages_payload = optional_object(input_value, "messages_payload")
    if messages_payload is not None:
        messages = spend_tracking_utils._get_messages_for_spend_logs_payload(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage, reportUnknownMemberType]
            standard_logging_payload=cast("StandardLoggingPayload", messages_payload)
        )
    return cast(
        Json,
        {
            "request": stored_request(input_value, litellm_params),
            "response": response,
            "messages": messages,
        },
    )


def classifier(input_value: dict[str, Json]) -> Json:
    configure(
        store_bodies=bool_value(input_value.get("store_bodies"), "store_bodies"),
        max_string_length=int_value(input_value.get("max_string_length"), "max_string_length"),
        turn_off_message_logging=bool_value(
            input_value.get("turn_off_message_logging"), "turn_off_message_logging"
        ),
    )
    litellm_params = object_value(input_value.get("litellm_params"), "litellm_params")
    standard = optional_object(input_value, "standard_logging_object")
    snapshots: list[Json] = []
    for spec in list_value(input_value.get("snapshots"), "snapshots"):
        spec_object = object_value(spec, "snapshot")
        snapshot = classifier_logging.classifier_input_snapshot(
            spec_object.get("value"),
            openai_sdk=bool_value(spec_object.get("openai_sdk"), "openai_sdk"),
        )
        snapshots.append(cast(Json, snapshot))
    audit: Json = (
        None
        if standard is None
        else cast(Json, classifier_logging.classifier_audit_fields(standard))
    )
    without: Json = (
        None
        if standard is None
        else cast(Json, classifier_logging.without_classifier_audit(standard))
    )
    return {
        "snapshots": snapshots,
        "audit": audit,
        "without": without,
        "request": stored_request(input_value, litellm_params),
    }


def vector_and_placeholder(input_value: dict[str, Json]) -> Json:
    configure(
        store_bodies=bool_value(input_value.get("store_bodies"), "store_bodies"),
        max_string_length=2048,
        turn_off_message_logging=False,
    )
    litellm_params = object_value(input_value.get("litellm_params"), "litellm_params")
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
        string_value(input_value.get("model_group"), "model_group"),
        string_value(input_value.get("raw_model"), "raw_model"),
    )
    return {"vector_store": plain(redacted), "placeholder": plain(placeholder)}


def session_value(case: dict[str, Json]) -> Json:
    metadata = optional_object(case, "metadata")
    configured_omit = bool_value(case.get("configured_omit"), "configured_omit")
    proxy_server.general_settings["missing_session_id"] = "omit" if configured_omit else None  # pyright: ignore[reportUnknownMemberType]
    trace_id = case.get("trace_id")
    litellm_trace_id = case.get("litellm_trace_id")
    injected_uuid = string_value(case.get("injected_uuid"), "injected_uuid")
    omit = spend_tracking_utils._omits_session_id_when_missing(metadata)  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
    batch = spend_tracking_utils._get_batch_trace_session_id(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
        optional_string(case, "call_type"), optional_string(case, "request_id")
    )
    kwargs: dict[str, object] = (
        {} if litellm_trace_id is None else {"litellm_trace_id": litellm_trace_id}
    )
    payload = (
        None
        if trace_id is None
        else cast("StandardLoggingPayload", {"trace_id": trace_id})
    )
    fixed = SimpleNamespace(uuid4=lambda: uuid.UUID(injected_uuid))
    with patch.object(litellm_uuid, "uuid", fixed):
        value = spend_tracking_utils._get_session_id_for_spend_log(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
            kwargs=kwargs,
            metadata=metadata,
            standard_logging_payload=payload,
            omit_when_missing=omit,
            batch_trace_session_id=batch,
        )
    return cast(Json, value)


def session(input_value: dict[str, Json]) -> Json:
    cases = list_value(input_value.get("sessions"), "sessions")
    values = [session_value(object_value(case, "session")) for case in cases]
    durations: list[int | None] = []
    for entry in list_value(input_value.get("durations"), "durations"):
        duration = object_value(entry, "duration")
        durations.append(
            spend_tracking_utils._get_request_duration_ms(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
                datetime.fromisoformat(string_value(duration.get("start"), "start")),
                datetime.fromisoformat(string_value(duration.get("end"), "end")),
            )
        )
    timestamps: list[int | None] = []
    for item in list_value(input_value.get("timestamps"), "timestamps"):
        moment = datetime.fromisoformat(string_value(item, "timestamp"))
        timestamps.append(
            epoch_micros(
                spend_tracking_utils._ensure_datetime_utc(moment)  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]
            )
        )
    return cast(Json, {"sessions": values, "durations": durations, "timestamps": timestamps})


def run(input_value: dict[str, Json]) -> Json:
    operation = string_value(input_value.get("operation"), "operation")
    if operation == "storage_policy":
        return storage_policy(input_value)
    if operation == "body_storage":
        return body_storage(input_value)
    if operation == "classifier":
        return classifier(input_value)
    if operation == "vector_and_placeholder":
        return vector_and_placeholder(input_value)
    if operation == "session":
        return session(input_value)
    raise ValueError(f"unknown spend_bodies operation ({litellm.__name__})")
