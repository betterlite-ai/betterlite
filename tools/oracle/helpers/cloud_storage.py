from typing import TypedDict
from urllib.parse import unquote

from litellm.litellm_core_utils.cloud_storage_security import (
    _validate_cloud_object_path,  # pyright: ignore[reportPrivateUsage] # adapter captures the pinned validator
    encode_gcs_object_name_for_url,
    encode_s3_object_key_for_url,
    is_managed_cloud_storage_uri,
    sanitize_cloud_object_component,
    sanitize_cloud_object_path,
    should_allow_legacy_cloud_file_ids,
    split_configured_cloud_bucket_name,
    validate_managed_cloud_file_id,
)

Json = None | bool | int | float | str | list["Json"] | dict[str, "Json"]


class Input(TypedDict):
    function: str
    value: str
    fallback: str
    params: dict[str, Json]
    file_id: str
    scheme: str
    bucket: str
    prefixes: list[str]
    legacy: bool


def failure(error: ValueError) -> Json:
    return {"class": type(error).__name__, "type": type(error).__name__, "message": str(error)}


def run(input_value: Input) -> Json:  # noqa: C901, PLR0911 # each named oracle operation captures a separate pinned seam
    function = input_value.get("function", "")
    try:
        if function == "cloud_uri_detection":
            return {"value": is_managed_cloud_storage_uri(input_value["value"])}
        if function == "object_component_sanitization":
            return {
                "value": sanitize_cloud_object_component(
                    input_value.get("value"), input_value.get("fallback", "file")
                )
            }
        if function == "object_path_sanitization":
            return {
                "value": sanitize_cloud_object_path(
                    input_value.get("value"), input_value.get("fallback", "file")
                )
            }
        if function == "cloud_object_encoding":
            value = input_value["value"]
            return {
                "gcs": encode_gcs_object_name_for_url(value),
                "s3": encode_s3_object_key_for_url(value),
            }
        if function == "configured_bucket_split":
            bucket, prefix = split_configured_cloud_bucket_name(input_value["value"])
            return {"bucket": bucket, "prefix": prefix}
        if function == "object_path_validation":
            _validate_cloud_object_path(input_value["value"])
            return {"ok": True}
        if function == "legacy_trusted_policy":
            return {"value": should_allow_legacy_cloud_file_ids(input_value.get("params"))}
        if function in {"managed_id_authorization", "legacy_prefix_boundary"}:
            bucket, object_name = validate_managed_cloud_file_id(
                input_value["file_id"],
                input_value["scheme"],
                input_value["bucket"],
                input_value.get("prefixes", []),
                input_value.get("legacy", False),
            )
            return {"bucket": bucket, "object": object_name}
        if function == "encoded_traversal_order":
            decoded = unquote(input_value["value"])
            scheme, path = (
                ("s3://", decoded[5:]) if decoded.startswith("s3://") else ("gs://", decoded[5:])
            )
            bucket, object_name = path.split("/", 1)
            return {"bucket": f"{scheme}{bucket}", "object": object_name}
        raise ValueError(f"unknown cloud storage function: {function}")
    except ValueError as error:
        return failure(error)
