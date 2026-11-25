from datetime import datetime
from typing import Final, Literal, TypeAlias, TypedDict

from litellm.proxy.vector_store_endpoints.management_endpoints import (
    _redact_sensitive_litellm_params,  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/vector_store_endpoints/management_endpoints.py#L92-L132
    _with_ownership,  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/vector_store_endpoints/management_endpoints.py#L81-L83
)
from pydantic import JsonValue, TypeAdapter

JSON: Final[TypeAdapter[JsonValue]] = TypeAdapter(JsonValue)


class VectorStore(TypedDict, total=False):
    vector_store_id: str
    custom_llm_provider: str
    vector_store_name: str | None
    vector_store_description: str | None
    vector_store_metadata: dict[str, JsonValue] | str | None
    created_at: datetime | None
    updated_at: datetime | None
    litellm_credential_name: str | None
    litellm_params: dict[str, JsonValue] | None
    team_id: str | None
    user_id: str | None
    is_config: bool


class OwnershipProjectionCall(TypedDict):
    op: Literal["ownership_projection"]
    vector_store: VectorStore


class RedactVectorCredentialsCall(TypedDict):
    op: Literal["redact_vector_credentials"]
    litellm_params: JsonValue
    depth: int


class SerializedVectorParamsCall(TypedDict):
    op: Literal["serialized_vector_params"]
    litellm_params: str
    depth: int


Call: TypeAlias = (
    OwnershipProjectionCall | RedactVectorCredentialsCall | SerializedVectorParamsCall
)


def project(vector_store: VectorStore) -> JsonValue:
    return JSON.validate_python(_with_ownership(vector_store))  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/vector_store_endpoints/management_endpoints.py#L81-L83


def redact(litellm_params: JsonValue, depth: int) -> JsonValue:
    return JSON.validate_python(_redact_sensitive_litellm_params(litellm_params, depth))  # type: ignore[misc]  # pyright: ignore[reportAny, reportUnknownArgumentType, reportUnknownVariableType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/vector_store_endpoints/management_endpoints.py#L92-L132


def run(input_value: Call) -> JsonValue:
    if input_value["op"] == "ownership_projection":
        return project(input_value["vector_store"])
    return redact(input_value["litellm_params"], input_value["depth"])
