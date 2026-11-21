from typing import Literal, TypeAlias, TypedDict

from litellm.types.rag import BedrockVectorStoreOptions


class PoliciesInput(TypedDict):
    step: Literal["policies"]
    vector: BedrockVectorStoreOptions
    environment: dict[str, str]
    unique_id: str
    account_id: str
    caller_arn: str


class CollectionInput(TypedDict):
    step: Literal["collection"]
    vector: BedrockVectorStoreOptions
    environment: dict[str, str]
    unique_id: str
    account_id: str
    caller_arn: str
    statuses: list[str]


class IndexInput(TypedDict):
    step: Literal["index"]
    vector: BedrockVectorStoreOptions
    environment: dict[str, str]
    unique_id: str
    collection_name: str
    endpoint: str
    failures: list[str]


class RoleInput(TypedDict):
    step: Literal["role"]
    vector: BedrockVectorStoreOptions
    environment: dict[str, str]
    unique_id: str
    account_id: str
    collection_arn: str


class FlowInput(TypedDict):
    step: Literal["flow"]
    vector: BedrockVectorStoreOptions
    environment: dict[str, str]
    unique_id: str


class StoreInput(TypedDict):
    step: Literal["store"]
    vector: BedrockVectorStoreOptions
    environment: dict[str, str]
    unique_id: str
    content: str | None
    filename: str | None
    content_type: str | None
    job_statuses: list[str]


Input: TypeAlias = (
    PoliciesInput | CollectionInput | IndexInput | RoleInput | FlowInput | StoreInput
)
