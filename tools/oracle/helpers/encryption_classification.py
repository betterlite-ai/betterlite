from typing import TypeAlias, cast

from litellm.proxy.common_utils.encrypt_decrypt_utils import (
    decrypt_value_helper,
)

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]


def classify(value: Json, key: str) -> str:
    if not isinstance(value, str):
        return "not-a-string"
    if value.startswith("v2:gcm:"):
        return "migrated"
    if not value:
        return "plaintext"
    decrypted = decrypt_value_helper(
        value=value, key=key, exception_type="debug", return_original_value=False
    )
    return "legacy" if decrypted is not None else "plaintext"


def run(input_value: dict[str, Json]) -> Json:
    values = input_value.get("values")
    if not isinstance(values, list):
        raise ValueError("values must be a list")
    key = input_value.get("key", "scan")
    if not isinstance(key, str):
        raise ValueError("key must be a string")
    counts: dict[str, int] = {}
    for value in values:
        category = classify(value, key)
        counts[category] = counts.get(category, 0) + 1
    return cast(Json, {"counts": counts})
