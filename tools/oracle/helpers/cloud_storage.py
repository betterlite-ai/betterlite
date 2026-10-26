import uuid
from types import MappingProxyType
from typing import TypedDict, cast
from unittest.mock import patch

from litellm.litellm_core_utils import cloud_storage_security as cloud
from litellm.litellm_core_utils.cloud_storage_security import (
    _validate_cloud_object_path,  # pyright: ignore[reportPrivateUsage] # owned validator
)

Json = None | bool | int | float | str | list["Json"] | dict[str, "Json"]


class Input(TypedDict):
    function: str
    value: Json
    fallback: str
    trusted: bool
    file_id: str
    scheme: str
    bucket: str
    prefixes: list[str]
    legacy: bool


def run(data: Input) -> Json:  # noqa: PLR0911 # the fixture dispatches the pinned helper functions
    value = data.get("value")
    fallback = data.get("fallback", "file")
    match data["function"]:
        case "cloud_uri_detection":
            return {"value": cloud.is_managed_cloud_storage_uri(cast(str, value))}
        case "object_component_sanitization":
            return {"value": cloud.sanitize_cloud_object_component(cast(str, value), fallback)}
        case "object_path_sanitization":
            with patch(
                "litellm.litellm_core_utils.cloud_storage_security.uuid.uuid4",
                return_value=uuid.UUID(int=0),
            ):
                return {
                    "value": cloud.sanitize_cloud_object_path(cast(str, value), fallback),
                    "name": cloud.build_managed_cloud_object_name(
                        "raw/", cast(str, value), fallback
                    ),
                }
        case "configured_bucket_split":
            bucket, prefix = cloud.split_configured_cloud_bucket_name(cast(str, value))
            return {"bucket": bucket, "prefix": prefix}
        case "object_path_validation":
            _validate_cloud_object_path(cast(str, value))
            return {"ok": True}
        case "cloud_object_encoding":
            return {
                "gcs": cloud.encode_gcs_object_name_for_url(cast(str, value)),
                "s3": cloud.encode_s3_object_key_for_url(cast(str, value)),
            }
        case "legacy_trusted_policy":
            settings = {"allow_legacy_cloud_file_ids": value}
            snapshot = MappingProxyType(settings) if data.get("trusted", False) else settings
            return {
                "value": cloud.should_allow_legacy_cloud_file_ids(
                    {
                        "_litellm_internal_model_credentials": snapshot,
                        "allow_legacy_cloud_file_ids": value,
                    }
                )
            }
        case "managed_id_authorization" | "legacy_prefix_boundary" | "encoded_traversal_order":
            bucket, name = cloud.validate_managed_cloud_file_id(
                data["file_id"],
                data["scheme"],
                data["bucket"],
                data.get("prefixes", []),
                data.get("legacy", False),
            )
            return {"bucket": bucket, "object": name}
        case _:
            raise ValueError("unknown cloud storage function")
