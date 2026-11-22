from typing import NotRequired, TypedDict

from litellm.proxy.management_endpoints import coordination_redis_endpoints
from pydantic import JsonValue, TypeAdapter

SETTINGS_ADAPTER = TypeAdapter(dict[str, JsonValue])


class Input(TypedDict):
    function: str
    settings: dict[str, JsonValue]
    saved: NotRequired[dict[str, JsonValue]]
    message: NotRequired[str]


def required_saved(input_value: Input) -> dict[str, JsonValue]:
    saved = input_value.get("saved")
    if saved is None:
        raise ValueError("the merge function needs a saved block")
    return saved


def required_message(input_value: Input) -> str:
    message = input_value.get("message")
    if message is None:
        raise ValueError("the scrub function needs a message")
    return message


def run(input_value: Input) -> JsonValue:
    function = input_value["function"]
    settings = input_value["settings"]
    if function == "redact":
        redacted = coordination_redis_endpoints._redact_credentials(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/management_endpoints/coordination_redis_endpoints.py#L87
            settings
        )
        return SETTINGS_ADAPTER.validate_python(redacted)
    if function == "merge":
        merged = coordination_redis_endpoints._merge_over_saved(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/management_endpoints/coordination_redis_endpoints.py#L125
            settings, required_saved(input_value)
        )
        return SETTINGS_ADAPTER.validate_python(merged)
    if function == "audit":
        audited = coordination_redis_endpoints._redact_all_values(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/management_endpoints/coordination_redis_endpoints.py#L95
            settings
        )
        return SETTINGS_ADAPTER.validate_python(audited)
    if function == "scrub":
        scrubbed = coordination_redis_endpoints._scrub_credentials(  # noqa: SLF001  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/management_endpoints/coordination_redis_endpoints.py#L112
            required_message(input_value), settings
        )
        return {"message": scrubbed}
    raise ValueError(f"unsupported coordination_settings function {function}")
