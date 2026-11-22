import logging
from collections.abc import Mapping
from typing import Final, Protocol, TypeAlias, runtime_checkable

from pydantic import JsonValue, TypeAdapter

import betterlite
from betterlite._completion.errors import UnsupportedCallArgument
from betterlite.llms.base_llm.base_utils import ResponseFormatModel, type_to_response_format_param
from betterlite.types.utils import Message

LOGGER: Final = logging.getLogger("LiteLLM")
JSON: Final[TypeAdapter[JsonValue]] = TypeAdapter(JsonValue)
JSON_OBJECT: Final[TypeAdapter[dict[str, JsonValue]]] = TypeAdapter(dict[str, JsonValue])
MESSAGES: Final[TypeAdapter[list[JsonValue]]] = TypeAdapter(list[JsonValue])


@runtime_checkable
class SessionLike(Protocol):
    ...


@runtime_checkable
class TimeoutValues(Protocol):
    connect: float | None
    read: float | None
    write: float | None
    pool: float | None


@runtime_checkable
class _ProxyAuthHeadersProvider(Protocol):
    def get_auth_headers(self) -> Mapping[str, str]: ...


CallValue: TypeAlias = JsonValue | SessionLike

# untyped betterlite globals (proxy_auth, config_path, num_retries) get a typed seam here
proxy_auth: _ProxyAuthHeadersProvider | None = None
config_path: JsonValue = None
num_retries: JsonValue = None


def _proxy_auth_headers() -> dict[str, JsonValue] | None:
    provider: _ProxyAuthHeadersProvider | None = betterlite.proxy_auth
    if provider is None:
        provider = proxy_auth
    if provider is None:
        return None
    try:
        headers: dict[str, str] = dict(provider.get_auth_headers())
        return JSON_OBJECT.validate_python(headers)
    except Exception as error:
        LOGGER.warning("Failed to get proxy auth headers: %s", error)
        return None


def message_value(message: dict[str, JsonValue] | Message) -> JsonValue:
    match message:
        case Message():
            return message.model_dump()
        case dict():
            return message


# ast-grep-ignore: no-one-line-functions
def messages_value(messages: list[dict[str, JsonValue] | Message]) -> JsonValue:
    return [message_value(message) for message in messages]


def timeout_value(value: CallValue | TimeoutValues) -> JsonValue:
    match value:
        case float() | int():
            return value
        case str():
            return float(value)
        case TimeoutValues():
            phases: dict[str, JsonValue] = {
                "connect": value.connect,
                "read": value.read,
                "write": value.write,
                "pool": value.pool,
            }
            return phases
        case object():
            raise UnsupportedCallArgument("timeout")


def response_format_value(value: JsonValue | type[ResponseFormatModel]) -> JsonValue:
    formatted = type_to_response_format_param(value)
    if formatted is None:
        raise UnsupportedCallArgument("response_format")
    return formatted


def settings_snapshot() -> dict[str, JsonValue]:
    settings: dict[str, JsonValue] = {
        "drop_params": JSON.validate_python(betterlite.drop_params),
        "model_fallbacks": JSON.validate_python(betterlite.model_fallbacks),
        "model_alias_map": JSON.validate_python(betterlite.model_alias_map),
        "num_retries": JSON.validate_python(betterlite.num_retries),
        "api_key": JSON.validate_python(betterlite.api_key),
        "api_base": JSON.validate_python(betterlite.api_base),
        "route_all_to_responses": JSON.validate_python(
            betterlite.route_all_chat_openai_to_responses
        ),
        "add_function_to_prompt": JSON.validate_python(betterlite.add_function_to_prompt),
        "disable_stop_sequence_limit": JSON.validate_python(betterlite.disable_stop_sequence_limit),
    }
    auth = _proxy_auth_headers()
    if auth is not None:
        settings["proxy_headers"] = auth
    return settings
