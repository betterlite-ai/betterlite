from collections.abc import Mapping
from typing import Final, Literal, TypeAlias

from betterlite._core_helpers.values import PythonValue

MetadataName: TypeAlias = Literal["metadata", "litellm_metadata"]
MetadataBucket: TypeAlias = dict[str, PythonValue]

_SPEND_MARKER: Final = "user_api_key"


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
        case object():
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
            if not params:
                return {}
            metadata = _bucket(params, "metadata")
            litellm_metadata = _bucket(params, "litellm_metadata")
            if litellm_metadata and metadata:
                return _with_spend_metadata(litellm_metadata, metadata)
            if litellm_metadata:
                return litellm_metadata
            if metadata:
                return metadata
            return {}
        case object():
            raise TypeError("litellm_params must be an object")


def _bucket(params: dict[str, PythonValue], name: MetadataName) -> MetadataBucket:
    value = params.get(name)
    if value is None:
        return {}
    match value:
        case dict() as fields:
            return fields
        case object():
            raise TypeError(f"{name} must be an object")


def _with_spend_metadata(
    litellm_metadata: MetadataBucket, metadata: MetadataBucket
) -> MetadataBucket:
    spend = {name: value for name, value in metadata.items() if _SPEND_MARKER in name}
    return {**litellm_metadata, **spend}


def reconstruct_model_name(
    model_name: str, custom_llm_provider: str | None, metadata: dict[str, PythonValue]
) -> str:
    deployment = metadata.get("deployment")
    if deployment is not None:
        match deployment:
            case str() as name:
                if "/" in name:
                    return name
            case object():
                raise TypeError("deployment must be a string")
    if custom_llm_provider == "bedrock" and len(model_name) > 0 and "/" not in model_name:
        return f"{custom_llm_provider}/{model_name}"
    return model_name
