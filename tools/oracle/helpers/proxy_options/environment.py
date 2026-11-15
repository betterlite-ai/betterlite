import os
from typing import Final

from pydantic import JsonValue, TypeAdapter

MANAGED_ENV: Final = (
    "LITELLM_PROXY_API_BASE",
    "LITELLM_PROXY_API_KEY",
    "USE_LITELLM_PROXY",
    "OPENAI_API_KEY",
)
VALUE: Final[TypeAdapter[JsonValue]] = TypeAdapter(JsonValue)


def install_environment(
    *,
    base: str | None,
    key: str | None,
    forced: bool | None,
    openai_key: str | None,
) -> None:
    for name in MANAGED_ENV:
        os.environ.pop(name, None)
    values: dict[str, str | None] = {
        "LITELLM_PROXY_API_BASE": base,
        "LITELLM_PROXY_API_KEY": key,
        "OPENAI_API_KEY": openai_key,
    }
    for name, value in values.items():
        if value is not None:
            os.environ[name] = value
    if forced is not None:
        os.environ["USE_LITELLM_PROXY"] = "true" if forced else "false"
