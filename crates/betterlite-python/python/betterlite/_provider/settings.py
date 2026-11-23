from typing import Final, TypedDict

from pydantic import JsonValue, TypeAdapter

import betterlite


class CustomProviderEntry(TypedDict):
    provider: str


CUSTOM_PROVIDERS: Final[TypeAdapter[list[CustomProviderEntry]]] = TypeAdapter(
    list[CustomProviderEntry]
)
JSON: Final[TypeAdapter[JsonValue]] = TypeAdapter(JsonValue)
MODEL_NAMES: Final[TypeAdapter[set[str]]] = TypeAdapter(set[str])


def provider_settings() -> str:
    # read at call time, so a later custom_provider_map assignment reaches the next lookup
    entries = CUSTOM_PROVIDERS.validate_python(betterlite.custom_provider_map)
    names = MODEL_NAMES.validate_python(betterlite.model_list_set)
    settings: dict[str, JsonValue] = {
        "custom_provider_map": JSON.validate_python(entries),
        "model_list_set": JSON.validate_python(sorted(names)),
        "api_key": JSON.validate_python(betterlite.api_key),
        "api_base": JSON.validate_python(betterlite.api_base),
        "xai_key": JSON.validate_python(betterlite.xai_key),
        "inception_key": JSON.validate_python(betterlite.inception_key),
        "lemonade_key": JSON.validate_python(betterlite.lemonade_key),
    }
    return JSON.dump_json(settings).decode()
