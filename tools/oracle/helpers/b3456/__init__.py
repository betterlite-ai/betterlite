import asyncio
import os
from typing import TypeAlias, cast
from unittest.mock import patch

from litellm.rag.ingestion import bedrock_ingestion
from litellm.rag.ingestion.bedrock_ingestion import BedrockRAGIngestion
from litellm.types.rag import RAGIngestOptions

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]
Params: TypeAlias = dict[str, Json]
Call: TypeAlias = tuple[str, Params]


def object_of(value: Json, name: str) -> Params:
    if not isinstance(value, dict):
        raise ValueError(f"{name} must be an object")
    return cast(Params, value)


def objects_of(value: Json, name: str) -> list[Params]:
    if not isinstance(value, list):
        raise ValueError(f"{name} must be a list")
    return [object_of(entry, name) for entry in value]


def text_of(value: Json, name: str) -> str:
    if not isinstance(value, str):
        raise ValueError(f"{name} must be a string")
    return value


def environment_of(value: Json) -> dict[str, str]:
    return cast(dict[str, str], object_of(value, "environment"))


class FakeAgent:
    def __init__(self) -> None:
        self.calls: list[Call] = []
        self.data_sources: list[Json] = []
        self.data_source: Params = {}
        self.knowledge_base_id = ""
        self.data_source_id = ""

    def list_data_sources(self, **params: Json) -> Params:
        self.calls.append(("list_data_sources", params))
        return {"dataSourceSummaries": list(self.data_sources)}

    def get_data_source(self, **params: Json) -> Params:
        self.calls.append(("get_data_source", params))
        return dict(self.data_source)

    def create_bucket(self, **params: Json) -> None:
        self.calls.append(("create_bucket", params))

    def create_knowledge_base(self, **params: Json) -> Params:
        self.calls.append(("create_knowledge_base", params))
        return {"knowledgeBase": {"knowledgeBaseId": self.knowledge_base_id, "status": "CREATING"}}

    def get_knowledge_base(self, **params: Json) -> Params:
        self.calls.append(("get_knowledge_base", params))
        return {"knowledgeBase": {"knowledgeBaseId": self.knowledge_base_id, "status": "ACTIVE"}}

    def create_data_source(self, **params: Json) -> Params:
        self.calls.append(("create_data_source", params))
        return {"dataSource": {"dataSourceId": self.data_source_id}}


class ClientProvider:
    def __init__(self, agent: FakeAgent) -> None:
        self.agent = agent

    def __call__(self, _service_name: str) -> FakeAgent:
        return self.agent


def build(vector: Json, environment: Json) -> BedrockRAGIngestion:
    options = cast(RAGIngestOptions, {"vector_store": object_of(vector, "vector")})
    with patch.dict(os.environ, environment_of(environment), clear=True):
        return BedrockRAGIngestion(ingest_options=options)


def call_params(calls: list[Call], name: str) -> Params:
    for call_name, params in calls:
        if call_name == name:
            return params
    raise ValueError(f"the {name} call was not recorded")


def location_constraint(calls: list[Call]) -> str | None:
    configuration = call_params(calls, "create_bucket").get("CreateBucketConfiguration")
    if not isinstance(configuration, dict):
        return None
    constraint = cast(Params, configuration).get("LocationConstraint")
    return constraint if isinstance(constraint, str) else None


def created(ingestion: BedrockRAGIngestion, name: str) -> str | None:
    value = ingestion._created_resources.get(name)  # noqa: SLF001 # pyright: ignore[reportPrivateUsage]
    return value if isinstance(value, str) else None


def principal(input_value: Params) -> Json:
    arns: list[Json] = []
    for case in objects_of(input_value["cases"], "cases"):
        caller_arn = text_of(case["caller_arn"], "caller_arn")
        account_id = text_of(case["account_id"], "account_id")
        arns.append(
            bedrock_ingestion._normalize_principal_arn(  # noqa: SLF001 # pyright: ignore[reportPrivateUsage]
                caller_arn, account_id
            )
        )
    return {"principal_arns": arns}


def detect(input_value: Params) -> Json:
    agent = FakeAgent()
    agent.data_sources = cast(list[Json], objects_of(input_value["data_sources"], "data_sources"))
    agent.data_source = object_of(input_value["data_source"], "data_source")
    ingestion = build(input_value["vector"], input_value.get("environment", {}))
    with patch.object(BedrockRAGIngestion, "_get_boto3_client", ClientProvider(agent)):
        ingestion._auto_detect_config()  # noqa: SLF001 # pyright: ignore[reportPrivateUsage]
    return {
        "data_source_id": ingestion.data_source_id,
        "s3_bucket": ingestion.s3_bucket,
        "s3_prefix": ingestion.s3_prefix,
    }


def bucket(input_value: Params) -> Json:
    results: list[Json] = []
    for case in objects_of(input_value["cases"], "cases"):
        agent = FakeAgent()
        ingestion = build(case["vector"], case.get("environment", {}))
        with patch.object(BedrockRAGIngestion, "_get_boto3_client", ClientProvider(agent)):
            name = ingestion._create_s3_bucket(  # noqa: SLF001 # pyright: ignore[reportPrivateUsage]
                text_of(case["unique_id"], "unique_id")
            )
        results.append(
            {
                "name": name,
                "location_constraint": location_constraint(agent.calls),
                "created": created(ingestion, "s3_bucket"),
            }
        )
    return {"buckets": results}


def create_knowledge_base(input_value: Params) -> Json:
    agent = FakeAgent()
    agent.knowledge_base_id = text_of(input_value["knowledge_base_id"], "knowledge_base_id")
    ingestion = build(input_value["vector"], input_value.get("environment", {}))
    with patch.object(BedrockRAGIngestion, "_get_boto3_client", ClientProvider(agent)):
        knowledge_base_id = asyncio.run(
            ingestion._create_knowledge_base(  # noqa: SLF001 # pyright: ignore[reportPrivateUsage]
                text_of(input_value["kb_name"], "kb_name"),
                text_of(input_value["role_arn"], "role_arn"),
                text_of(input_value["collection_arn"], "collection_arn"),
            )
        )
    return {
        "knowledge_base_id": knowledge_base_id,
        "created": created(ingestion, "knowledge_base"),
        "create_params": call_params(agent.calls, "create_knowledge_base"),
        "get_calls": [params for name, params in agent.calls if name == "get_knowledge_base"],
    }


def create_data_source(input_value: Params) -> Json:
    agent = FakeAgent()
    agent.data_source_id = text_of(input_value["data_source_id"], "data_source_id")
    vector = object_of(input_value["vector"], "vector")
    ingestion = build(vector, input_value.get("environment", {}))
    bucket_name = vector.get("s3_bucket")
    ingestion.s3_bucket = bucket_name if isinstance(bucket_name, str) else None
    with patch.object(BedrockRAGIngestion, "_get_boto3_client", ClientProvider(agent)):
        data_source_id = ingestion._create_data_source(  # noqa: SLF001 # pyright: ignore[reportPrivateUsage]
            text_of(input_value["kb_name"], "kb_name")
        )
    return {
        "data_source_id": data_source_id,
        "created": created(ingestion, "data_source"),
        "create_params": call_params(agent.calls, "create_data_source"),
    }


def run(input_value: dict[str, Json]) -> Json:
    step = text_of(input_value.get("step"), "step")
    match step:
        case "principal":
            return principal(input_value)
        case "detect":
            return detect(input_value)
        case "bucket":
            return bucket(input_value)
        case "create_knowledge_base":
            return create_knowledge_base(input_value)
        case "create_data_source":
            return create_data_source(input_value)
        case _:
            raise ValueError(f"unknown step: {step}")
