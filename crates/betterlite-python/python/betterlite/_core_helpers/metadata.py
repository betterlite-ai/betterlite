from collections.abc import Iterable, Mapping
from datetime import datetime
from typing import Final, Literal, TypeAlias

from betterlite._core_helpers.values import CallableLeaf, PythonValue, object_fields, text_of
from betterlite.types.utils.base import Dumpable

MetadataName: TypeAlias = Literal["metadata", "litellm_metadata"]
MetadataBucket: TypeAlias = dict[str, PythonValue]

_SPEND_MARKER: Final = "user_api_key"
_RETRY_COUNT_KEY: Final = "request_retry_count"
_OAUTH_STAMP_KEY: Final = "used_client_oauth_token"


def get_metadata_variable_name_from_kwargs(  # ast-grep-ignore: no-long-names
    kwargs: Mapping[str, PythonValue],
) -> MetadataName:
    if "litellm_metadata" in kwargs:
        return "litellm_metadata"
    return "metadata"


def get_or_create_metadata_bucket(
    request_data: dict[str, PythonValue],
) -> tuple[MetadataName, MetadataBucket]:
    metadata_key = get_metadata_variable_name_from_kwargs(request_data)
    bucket = object_fields(request_data.get(metadata_key))
    if bucket is not None:
        return metadata_key, bucket
    created: MetadataBucket = {}
    request_data[metadata_key] = created
    return metadata_key, created


def get_litellm_metadata_from_kwargs(  # ast-grep-ignore: no-long-names
    kwargs: dict[str, PythonValue],
) -> MetadataBucket:
    litellm_params = kwargs.get("litellm_params")
    if litellm_params is None:
        return {}
    params = object_fields(litellm_params)
    if params is None:
        raise TypeError("litellm_params must be an object")
    metadata = _bucket(params, "metadata")
    litellm_metadata = _bucket(params, "litellm_metadata")
    if litellm_metadata and metadata:
        return add_missing_spend_metadata_to_litellm_metadata(litellm_metadata, metadata)
    if litellm_metadata:
        return litellm_metadata
    return metadata


def _bucket(params: dict[str, PythonValue], name: MetadataName) -> MetadataBucket:
    value = params.get(name)
    fields = object_fields(value)
    if fields is not None:
        return fields
    if value is not None:
        raise TypeError(f"{name} must be an object")
    return {}


def add_missing_spend_metadata_to_litellm_metadata(  # ast-grep-ignore: no-long-names
    litellm_metadata: MetadataBucket, metadata: MetadataBucket
) -> MetadataBucket:
    for key, value in metadata.items():
        if _SPEND_MARKER in key:
            litellm_metadata[key] = value
    return litellm_metadata


def max_retries_per_request_hit(
    kwargs: Mapping[str, PythonValue], num_retries_per_request: int | None
) -> bool:
    if num_retries_per_request is None:
        return False
    metadata = object_fields(kwargs.get(get_metadata_variable_name_from_kwargs(kwargs)))
    if metadata is None:
        return False
    return _retry_count_hit(metadata.get(_RETRY_COUNT_KEY), num_retries_per_request)


def _retry_count_hit(retry_count: PythonValue, limit: int) -> bool:
    match retry_count:
        case int() as count if count.__class__ is int:
            return count > 0 and limit <= count
        case (
            int() | float() | BaseException() | datetime()
            | Dumpable() | Iterable() | CallableLeaf() | None
        ):
            return False


def proxy_stamped_used_client_oauth_token(  # ast-grep-ignore: no-long-names
    metadata: PythonValue, litellm_params: Mapping[str, PythonValue] | None
) -> PythonValue:
    stamped: PythonValue = None
    if litellm_params is not None:
        stamped = litellm_params.get("litellm_metadata")
    stamped_fields = object_fields(stamped)
    if stamped_fields is not None and _OAUTH_STAMP_KEY in stamped_fields:
        return stamped_fields[_OAUTH_STAMP_KEY]
    metadata_fields = object_fields(metadata)
    if metadata_fields is None:
        return None
    return metadata_fields.get(_OAUTH_STAMP_KEY)


def reconstruct_model_name(
    model_name: str, custom_llm_provider: str | None, metadata: dict[str, PythonValue]
) -> str:
    deployment = metadata.get("deployment")
    if deployment is not None:
        name = text_of(deployment)
        if name is None:
            raise TypeError("deployment must be a string")
        if "/" in name:
            return name
    if custom_llm_provider == "bedrock" and len(model_name) > 0 and "/" not in model_name:
        return f"{custom_llm_provider}/{model_name}"
    return model_name
