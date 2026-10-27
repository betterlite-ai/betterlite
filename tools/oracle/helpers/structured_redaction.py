from typing import TypeAlias, cast

from litellm.litellm_core_utils.sensitive_data_masker import (
    mask_credentials_in_payload,
    mask_sensitive_keys,
    mask_sensitive_structure,
    redact_credentials_in_payload,
)

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]


def run(input_value: dict[str, Json]) -> Json:
    function = input_value.get("function")
    data = input_value.get("data")
    if function == "mask_sensitive_structure":
        return cast(Json, mask_sensitive_structure(data))
    if function == "mask_credentials_in_payload":
        return cast(Json, mask_credentials_in_payload(data))
    if function == "redact_credentials_in_payload":
        if not isinstance(data, dict):
            raise ValueError("data must be an object")
        return cast(Json, redact_credentials_in_payload(data))
    if function == "mask_sensitive_keys":
        if not isinstance(data, dict):
            raise ValueError("data must be an object")
        fields = input_value.get("sensitive_fields")
        if not isinstance(fields, list) or not all(isinstance(field, str) for field in fields):
            raise ValueError("sensitive_fields must be a list of strings")
        names = {field for field in fields if isinstance(field, str)}
        return cast(Json, mask_sensitive_keys(data, names))
    raise ValueError(f"unknown structured_redaction function: {function}")
