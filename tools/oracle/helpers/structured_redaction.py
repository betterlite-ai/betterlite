import json
from collections.abc import Callable
from typing import TypeAlias, cast

from litellm.litellm_core_utils.sensitive_data_masker import (
    SensitiveDataMasker,
    mask_credentials_in_payload,
    mask_sensitive_keys,
    mask_sensitive_structure,
    redact_credentials_in_payload,
)

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]


def names(value: Json) -> set[str]:
    if not isinstance(value, list) or not all(isinstance(item, str) for item in value):
        raise ValueError("field names must be strings")
    return {item for item in value if isinstance(item, str)}


def class_call(function: Json, data: Json, excluded: Json) -> Json:
    masker = SensitiveDataMasker()
    if function == "mask":
        return cast(Json, masker.mask(data))
    if function == "mask_value" and isinstance(data, str):
        method = "_mask_value"
        mask_value = cast(Callable[[str], str], getattr(masker, method))
        return mask_value(data)
    excluded_keys = names(excluded)
    if function == "is_sensitive_key" and isinstance(data, str):
        return masker.is_sensitive_key(data, excluded_keys)
    if function == "mask_dict" and isinstance(data, dict):
        return cast(Json, masker.mask_dict(data, excluded_keys=excluded_keys))
    raise ValueError("unexpected data type")


def call(input_value: dict[str, Json]) -> Json:
    function = input_value.get("function")
    data = input_value.get("data")
    if function in ("mask", "mask_value", "is_sensitive_key", "mask_dict"):
        return class_call(function, data, input_value.get("excluded_keys", []))
    if function == "mask_sensitive_structure":
        return cast(Json, mask_sensitive_structure(data))
    if function == "mask_credentials_in_payload":
        return cast(Json, mask_credentials_in_payload(data))
    if function == "redact_credentials_in_payload":
        return redact_call(data, input_value.get("data_json"))
    if not isinstance(data, dict):
        raise ValueError("data must be an object")
    if function == "mask_sensitive_keys":
        return cast(Json, mask_sensitive_keys(data, names(input_value.get("sensitive_fields"))))
    raise ValueError("unknown structured_redaction function")


def redact_call(data: Json, encoded: Json) -> Json:
    if encoded is not None:
        if data is not None or not isinstance(encoded, str):
            raise ValueError("select one payload representation")
        try:
            payload = cast(Json, json.loads(encoded))
        except json.JSONDecodeError as error:
            raise ValueError("invalid JSON payload") from error
    else:
        payload = data
    if not isinstance(payload, dict):
        raise ValueError("data must be an object")
    return cast(Json, redact_credentials_in_payload(payload))


def run(input_value: dict[str, Json]) -> Json:
    result = call(input_value)
    return {"result": result}
