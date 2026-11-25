from collections.abc import Iterator, Mapping, Sequence
from datetime import datetime
from typing import Final, Literal, TypeAlias

from betterlite._core_helpers.values import PythonValue
from betterlite.types.utils.base import Dumpable

MetadataName: TypeAlias = Literal["metadata", "litellm_metadata"]
MetadataBucket: TypeAlias = dict[str, PythonValue]

_SPEND_MARKER: Final = "user_api_key"
_RETRY_COUNT_KEY: Final = "request_retry_count"
_CLIENT_TOKEN_KEY: Final = "used_client_oauth_token"


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
    match request_data.get(metadata_key):
        case dict() as bucket:
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
    match litellm_params:
        case dict() as params:
            return _merged_metadata(params)
    raise TypeError("litellm_params must be an object")


def _merged_metadata(params: dict[str, PythonValue]) -> MetadataBucket:
    if not params:
        return {}
    metadata = _bucket(params, "metadata")
    litellm_metadata = _bucket(params, "litellm_metadata")
    if litellm_metadata and metadata:
        return add_missing_spend_metadata_to_litellm_metadata(litellm_metadata, metadata)
    if litellm_metadata:
        return litellm_metadata
    if metadata:
        return metadata
    return {}


def _bucket(params: dict[str, PythonValue], name: MetadataName) -> MetadataBucket:
    value = params.get(name)
    if value is None:
        return {}
    match value:
        case dict() as fields:
            return fields
    raise TypeError(f"{name} must be an object")


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
    match kwargs.get(get_metadata_variable_name_from_kwargs(kwargs)):
        case Mapping() as metadata:
            return _retry_count_hit(metadata.get(_RETRY_COUNT_KEY), num_retries_per_request)
    return False


def _retry_count_hit(retry_count: PythonValue, limit: int) -> bool:
    match retry_count:
        case int() as count if count.__class__ is int:
            return 0 < count and limit <= count
    return False


def proxy_stamped_used_client_oauth_token(  # ast-grep-ignore: no-long-names
    metadata: PythonValue, litellm_params: Mapping[str, PythonValue] | None
) -> PythonValue:
    stamped: PythonValue = None
    if litellm_params is not None:
        stamped = litellm_params.get("litellm_metadata")
    match stamped:
        case Mapping() as stamped_fields:
            if _CLIENT_TOKEN_KEY in stamped_fields:
                return stamped_fields[_CLIENT_TOKEN_KEY]
    match metadata:
        case Mapping() as metadata_fields:
            return metadata_fields.get(_CLIENT_TOKEN_KEY)
    return None


def reconstruct_model_name(
    model_name: str, custom_llm_provider: str | None, metadata: dict[str, PythonValue]
) -> str:
    deployment = metadata.get("deployment")
    if deployment is not None:
        match deployment:
            case str() as name:
                if "/" in name:
                    return name
            case bool() | int() | float() | BaseException() | datetime() | Dumpable():
                raise TypeError("deployment must be a string")
            case Iterator() | Sequence() | Mapping():
                raise TypeError("deployment must be a string")
        if callable(deployment):
            raise TypeError("deployment must be a string")
    if custom_llm_provider == "bedrock" and len(model_name) > 0 and "/" not in model_name:
        return f"{custom_llm_provider}/{model_name}"
    return model_name
