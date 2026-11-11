import asyncio
import contextlib
import os
from dataclasses import dataclass, field
from typing import Literal, NotRequired, Protocol, TypeAlias, TypedDict, Unpack, cast

from litellm.rag.ingestion.bedrock_ingestion import BedrockRAGIngestion
from litellm.types.rag import RAGIngestOptions
from pydantic import TypeAdapter
from typing_extensions import TypedDict as ParsedDict
from typing_extensions import override

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]


class PrincipalCase(TypedDict):
    caller_arn: str
    account_id: str


class AccessPolicy(ParsedDict):
    Principal: list[str]


class Principal(TypedDict):
    step: Literal["principal"]
    cases: list[PrincipalCase]


class Detect(TypedDict):
    step: Literal["detect"]
    vector: dict[str, Json]
    environment: dict[str, str]
    data_sources: list[Json]
    data_source: dict[str, Json]


class BucketCase(TypedDict):
    vector: dict[str, Json]
    environment: dict[str, str]
    unique_id: str


class Bucket(TypedDict):
    step: Literal["bucket"]
    cases: list[BucketCase]


class CreateKnowledgeBase(TypedDict):
    step: Literal["create_knowledge_base"]
    vector: dict[str, Json]
    environment: dict[str, str]
    kb_name: str
    knowledge_base_id: str
    role_arn: str
    collection_arn: str


class CreateDataSource(TypedDict):
    step: Literal["create_data_source"]
    vector: dict[str, Json]
    environment: dict[str, str]
    kb_name: str
    data_source_id: str


Input: TypeAlias = Principal | Detect | Bucket | CreateKnowledgeBase | CreateDataSource


class BucketConfiguration(TypedDict):
    LocationConstraint: str


class BucketParams(TypedDict):
    Bucket: str
    CreateBucketConfiguration: NotRequired[BucketConfiguration]


class CollectionCreatedError(Exception):
    pass


class BedrockClient(Protocol):
    def create_security_policy(self, **_params: Json) -> dict[str, Json]: ...

    def create_access_policy(self, **params: str) -> dict[str, Json]: ...

    def create_collection(self, **_params: Json) -> dict[str, Json]: ...

    def list_data_sources(self, **_params: Json) -> dict[str, Json]: ...

    def get_data_source(self, **_params: Json) -> dict[str, Json]: ...

    def create_bucket(self, **params: Unpack[BucketParams]) -> dict[str, Json]: ...

    def create_knowledge_base(self, **params: Json) -> dict[str, Json]: ...

    def get_knowledge_base(self, **params: Json) -> dict[str, Json]: ...

    def create_data_source(self, **params: Json) -> dict[str, Json]: ...


@dataclass
class FakeClient(BedrockClient):
    data_sources: list[Json] = field(default_factory=list)
    data_source: dict[str, Json] = field(default_factory=dict)
    knowledge_base_id: str = ""
    data_source_id: str = ""
    location_constraint: str | None = None
    knowledge_base_params: dict[str, Json] = field(default_factory=dict)
    data_source_params: dict[str, Json] = field(default_factory=dict)
    polls: list[Json] = field(default_factory=list)
    access_policy: str = ""

    @override
    def create_security_policy(self, **_params: Json) -> dict[str, Json]:
        return {}

    @override
    def create_access_policy(self, **params: str) -> dict[str, Json]:
        self.access_policy = params["policy"]
        return {}

    @override
    def create_collection(self, **_params: Json) -> dict[str, Json]:
        raise CollectionCreatedError

    @override
    def list_data_sources(self, **_params: Json) -> dict[str, Json]:
        return {"dataSourceSummaries": self.data_sources}

    @override
    def get_data_source(self, **_params: Json) -> dict[str, Json]:
        return self.data_source

    @override
    def create_bucket(self, **params: Unpack[BucketParams]) -> dict[str, Json]:
        configuration = params.get("CreateBucketConfiguration")
        self.location_constraint = (
            None if configuration is None else configuration["LocationConstraint"]
        )
        return {"Location": f"/{params['Bucket']}"}

    @override
    def create_knowledge_base(self, **params: Json) -> dict[str, Json]:
        self.knowledge_base_params = params
        return {"knowledgeBase": {"knowledgeBaseId": self.knowledge_base_id, "status": "CREATING"}}

    @override
    def get_knowledge_base(self, **params: Json) -> dict[str, Json]:
        self.polls.append(params)
        return {"knowledgeBase": {"knowledgeBaseId": self.knowledge_base_id, "status": "ACTIVE"}}

    @override
    def create_data_source(self, **params: Json) -> dict[str, Json]:
        self.data_source_params = params
        return {"dataSource": {"dataSourceId": self.data_source_id}}


class RecordingIngestion(BedrockRAGIngestion):
    def __init__(
        self, vector: dict[str, Json], environment: dict[str, str], client: FakeClient
    ) -> None:
        self.client = client
        # the constructor reads os.environ, so only the case environment may be visible
        saved = dict(os.environ)
        os.environ.clear()
        os.environ.update(environment)
        try:
            options = cast(  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/rag.py#L183
                RAGIngestOptions, {"vector_store": vector}
            )
            super().__init__(ingest_options=options)
        finally:
            os.environ.clear()
            os.environ.update(saved)

    @override
    def _get_boto3_client(self, service_name: str) -> FakeClient:
        return self.client

    def principal(self, caller_arn: str, account_id: str) -> str:
        root = f"arn:aws:iam::{account_id}:root"
        with contextlib.suppress(CollectionCreatedError):
            asyncio.run(self._create_opensearch_collection("oracle", account_id, caller_arn))
        policies = TypeAdapter(list[AccessPolicy]).validate_json(self.client.access_policy)
        principal = root
        for candidate in policies[0]["Principal"]:
            if candidate != root:
                principal = candidate
        return principal

    def detect(self) -> dict[str, Json]:
        self._auto_detect_config()  # type: ignore[no-untyped-call]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/rag/ingestion/bedrock_ingestion.py#L141
        return {
            "data_source_id": self.data_source_id,
            "s3_bucket": self.s3_bucket,
            "s3_prefix": self.s3_prefix,
        }

    def bucket(self, unique_id: str) -> dict[str, Json]:
        name = self._create_s3_bucket(unique_id)
        return {
            "name": name,
            "location_constraint": self.client.location_constraint,
            "created": str(self._created_resources["s3_bucket"]),
        }

    def knowledge_base(self, kb_name: str, role_arn: str, collection_arn: str) -> dict[str, Json]:
        knowledge_base_id = asyncio.run(
            self._create_knowledge_base(kb_name, role_arn, collection_arn)
        )
        return {
            "knowledge_base_id": knowledge_base_id,
            "created": str(self._created_resources["knowledge_base"]),
            "create_params": self.client.knowledge_base_params,
            "get_calls": self.client.polls,
        }

    def data_source_from(self, kb_name: str, bucket: Json) -> dict[str, Json]:
        match bucket:
            case str() as configured:
                self.s3_bucket = configured
            case None | bool() | int() | float() | list() | dict():
                self.s3_bucket = None
        data_source_id = self._create_data_source(kb_name)
        return {
            "data_source_id": data_source_id,
            "created": str(self._created_resources["data_source"]),
            "create_params": self.client.data_source_params,
        }


def run(input_value: Input) -> Json:
    if input_value["step"] == "principal":
        arns: list[Json] = []
        for identity in input_value["cases"]:
            ingestion = RecordingIngestion({}, {}, FakeClient())
            arns.append(ingestion.principal(identity["caller_arn"], identity["account_id"]))
        return {"principal_arns": arns}
    if input_value["step"] == "detect":
        client = FakeClient(
            data_sources=input_value["data_sources"], data_source=input_value["data_source"]
        )
        return RecordingIngestion(
            input_value["vector"], input_value["environment"], client
        ).detect()
    if input_value["step"] == "bucket":
        buckets: list[Json] = []
        for case in input_value["cases"]:
            bucket_client = FakeClient()
            ingestion = RecordingIngestion(case["vector"], case["environment"], bucket_client)
            buckets.append(ingestion.bucket(case["unique_id"]))
        return {"buckets": buckets}
    if input_value["step"] == "create_knowledge_base":
        kb_client = FakeClient(knowledge_base_id=input_value["knowledge_base_id"])
        ingestion = RecordingIngestion(input_value["vector"], input_value["environment"], kb_client)
        return ingestion.knowledge_base(
            input_value["kb_name"], input_value["role_arn"], input_value["collection_arn"]
        )
    source_client = FakeClient(data_source_id=input_value["data_source_id"])
    ingestion = RecordingIngestion(input_value["vector"], input_value["environment"], source_client)
    return ingestion.data_source_from(
        input_value["kb_name"], input_value["vector"].get("s3_bucket")
    )
