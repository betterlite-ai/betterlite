from types import SimpleNamespace
from typing import TYPE_CHECKING, TypeAlias, cast

import litellm
from litellm.proxy.guardrails.guardrail_hooks.straiker import straiker
from litellm.types.guardrails import GuardrailEventHooks, Mode
from litellm.types.proxy.guardrails.guardrail_hooks.straiker import StraikerWebhookUsage
from litellm.types.utils import ModelResponse

# ruff: noqa: SLF001
# pyright: reportPrivateUsage=false
Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]

# an unknown model makes get_llm_provider print to stdout, which corrupts the helper JSON.
litellm.suppress_debug_info = True

if TYPE_CHECKING:
    from litellm.litellm_core_utils.litellm_logging import Logging as LiteLLMLoggingObj


def object_of(value: Json) -> dict[str, Json]:
    if not isinstance(value, dict):
        raise TypeError("expected an object")
    return value


def hook_of(value: Json) -> object:
    # The wire value is the enum member's own name, not its str() repr.
    if isinstance(value, str):
        try:
            return GuardrailEventHooks(value)
        except ValueError:
            return value
    return value


def event_hook_of(value: Json) -> object:
    if isinstance(value, list):
        return [hook_of(item) for item in value]
    if isinstance(value, dict):
        return Mode.model_validate(value)
    return hook_of(value)


def identity(fields: dict[str, Json]) -> Json:
    identity_value = straiker._extract_identity(  # pyright: ignore[reportUnknownMemberType]
        object_of(fields["request_data"])
    )
    return cast(Json, identity_value.model_dump(mode="json", exclude_none=True))


def webhook_metadata(fields: dict[str, Json]) -> Json:
    request_data = object_of(fields["request_data"])
    defaults = cast(dict[str, str], object_of(fields["default_metadata"]))
    return cast(Json, straiker._build_webhook_metadata(  # pyright: ignore[reportUnknownMemberType]
        request_data, defaults
    ))


def provider(fields: dict[str, Json]) -> Json:
    request_data = object_of(fields["request_data"])
    model = cast(str | None, fields.get("model"))
    return straiker._resolve_provider(  # pyright: ignore[reportUnknownMemberType]
        request_data, model
    )


def destination(fields: dict[str, Json]) -> Json:
    return straiker._resolve_destination(  # pyright: ignore[reportUnknownMemberType]
        object_of(fields["request_data"])
    )


def call_surface(fields: dict[str, Json]) -> Json:
    request_data = object_of(fields["request_data"])
    logging_field = fields.get("logging_obj")
    logging_obj: object = None
    if isinstance(logging_field, dict):
        logging_obj = SimpleNamespace(call_type=logging_field.get("call_type"))
    typed = cast("LiteLLMLoggingObj | None", logging_obj)
    return straiker._resolve_call_surface(typed, request_data)  # pyright: ignore[reportUnknownMemberType]


def finish_reason(fields: dict[str, Json]) -> Json:
    return straiker._response_finish_reason(fields["response"])


def finish_reason_object(fields: dict[str, Json]) -> Json:
    response = ModelResponse(**cast(dict[str, object], object_of(fields["response"])))
    return straiker._response_finish_reason(response)


def usage(fields: dict[str, Json]) -> Json:
    usage_value = straiker._build_usage(fields["response"])
    if usage_value is None:
        return None
    return cast(Json, usage_value.model_dump(mode="json", exclude_none=True))


def configured_modes(fields: dict[str, Json]) -> Json:
    return cast(Json, straiker._configured_modes(event_hook_of(fields.get("event_hook"))))


def structured_messages(fields: dict[str, Json]) -> Json:
    return cast(
        Json,
        straiker._request_structured_messages(  # pyright: ignore[reportUnknownMemberType]
            object_of(fields["request_data"])
        ),
    )


def jsonable_item(item: Json) -> Json:
    match item:
        case {"plain": value}:
            return cast(Json, straiker._jsonable_dict(value))
        case {"model": fields}:
            record = StraikerWebhookUsage.model_validate(object_of(fields))
            return cast(Json, straiker._jsonable_dict(record))
        case _:
            raise ValueError("a jsonable item is plain or model")


def jsonable(fields: dict[str, Json]) -> Json:
    return jsonable_item(fields["item"])


def opaque_dict_list(fields: dict[str, Json]) -> Json:
    items = fields["items"]
    if not isinstance(items, list):
        raise TypeError("items must be a list")
    return cast(Json, straiker._opaque_dict_list([jsonable_item(item) for item in items]))


def streamed(fields: dict[str, Json]) -> Json:
    return straiker._is_streamed_request(  # pyright: ignore[reportUnknownMemberType]
        object_of(fields["request_data"])
    )


def status_failure(fields: dict[str, Json]) -> Json:
    status = fields["status"]
    text = fields["text"]
    if not isinstance(status, int) or isinstance(status, bool):
        raise TypeError("status must be an integer")
    if not isinstance(text, str):
        raise TypeError("text must be a string")
    failure = straiker._status_failure(status, text)
    return {
        "message": failure.message,
        "is_unreachable": failure.is_unreachable,
        "retryable": failure.retryable,
    }


def run(input_value: dict[str, Json]) -> Json:  # noqa: C901, PLR0911, PLR0912 - one branch per operation
    [(operation, request)] = input_value.items()
    fields = object_of(request)
    match operation:
        case "identity":
            return identity(fields)
        case "webhook_metadata":
            return webhook_metadata(fields)
        case "provider":
            return provider(fields)
        case "destination":
            return destination(fields)
        case "call_surface":
            return call_surface(fields)
        case "finish_reason":
            return finish_reason(fields)
        case "finish_reason_object":
            return finish_reason_object(fields)
        case "usage":
            return usage(fields)
        case "configured_modes":
            return configured_modes(fields)
        case "structured_messages":
            return structured_messages(fields)
        case "jsonable":
            return jsonable(fields)
        case "opaque_dict_list":
            return opaque_dict_list(fields)
        case "streamed":
            return streamed(fields)
        case "status_failure":
            return status_failure(fields)
        case _:
            raise ValueError(f"unsupported operation {operation}")
