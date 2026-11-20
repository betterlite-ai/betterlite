import asyncio
import os
from typing import Final
from unittest.mock import AsyncMock

from litellm.types.rag import BedrockVectorStoreOptions, RAGIngestOptions
from pydantic import JsonValue

from .fakes import (
    DOCUMENT,
    INDEX,
    FakeClient,
    FakeIndexError,
    RecordingIngestion,
    Scripted,
    install_fakes,
)
from .inputs import (
    CollectionInput,
    FlowInput,
    IndexInput,
    Input,
    PoliciesInput,
    RoleInput,
    StoreInput,
)

DETECTED_SOURCE: Final[dict[str, JsonValue]] = {
    "dataSource": {
        "dataSourceId": "ds1",
        "dataSourceConfiguration": {
            "s3Configuration": {
                "bucketArn": "arn:aws:s3:::detected",
                "inclusionPrefixes": ["kb/"],
            }
        },
    }
}


def build(
    vector: BedrockVectorStoreOptions, environment: dict[str, str], client: FakeClient
) -> RecordingIngestion:
    saved = dict(os.environ)
    os.environ.clear()
    os.environ.update(environment)
    try:
        ingestion = RecordingIngestion(
            ingest_options=RAGIngestOptions(vector_store=vector)
        )
    finally:
        os.environ.clear()
        os.environ.update(saved)
    ingestion.client = client
    return ingestion


def prepared_ingestion(
    vector: BedrockVectorStoreOptions, environment: dict[str, str], failures: list[str]
) -> tuple[FakeClient, RecordingIngestion]:
    client = scripted_client()
    install_fakes(failures, client.scripted.operations)
    return client, build(vector, environment, client)


def content_type_value(value: str) -> str | None:
    if value == "":
        return None
    return value


def scripted_client() -> FakeClient:
    return FakeClient(
        Scripted(
            knowledge_base_id="kb1",
            data_source_id="ds1",
            data_sources=[{"dataSourceId": "ds1"}],
            data_source=DETECTED_SOURCE,
        )
    )


def policies(input_value: PoliciesInput) -> dict[str, JsonValue]:
    client = scripted_client()
    client.scripted.statuses = ["ACTIVE"]
    ingestion = build(input_value["vector"], input_value["environment"], client)
    name, _arn = ingestion.collection(
        input_value["unique_id"], input_value["account_id"], input_value["caller_arn"]
    )
    recorded = client.scripted.policies
    return {
        "encryption": DOCUMENT.validate_json(recorded[0]),
        "network": DOCUMENT.validate_json(recorded[1]),
        "access": DOCUMENT.validate_json(recorded[2]),
        "collection": name,
    }


def collection(input_value: CollectionInput) -> dict[str, JsonValue]:
    client = scripted_client()
    client.scripted.statuses = list(input_value["statuses"])
    ingestion = build(input_value["vector"], input_value["environment"], client)
    try:
        name, arn = ingestion.collection(
            input_value["unique_id"], input_value["account_id"], input_value["caller_arn"]
        )
    except TimeoutError:
        return {
            "outcome": "timeout",
            "created": ingestion.created("opensearch_collection"),
            "operations": list(client.scripted.operations),
        }
    return {
        "outcome": "created",
        "name": name,
        "arn": arn,
        "created": ingestion.created("opensearch_collection"),
        "operations": list(client.scripted.operations),
    }


def index(input_value: IndexInput) -> dict[str, JsonValue]:
    client, ingestion = prepared_ingestion(
        input_value["vector"], input_value["environment"], list(input_value["failures"])
    )
    client.scripted.statuses = ["ACTIVE"]
    client.scripted.collection_endpoint = input_value["endpoint"]
    outcome = "created"
    try:
        ingestion.flow()
    except FakeIndexError:
        outcome = "raised"
    except RuntimeError:
        outcome = "exhausted"
    return {
        "outcome": outcome,
        "attempts": INDEX.attempts,
        "index_name": INDEX.index_name,
        "index_body": INDEX.body,
    }


def role(input_value: RoleInput) -> dict[str, JsonValue]:
    unique_id = input_value["unique_id"]
    account_id = input_value["account_id"]
    client = scripted_client()
    client.scripted.role_arn = f"arn:aws:iam::{account_id}:role/litellm-bedrock-kb-{unique_id}"
    ingestion = build(input_value["vector"], input_value["environment"], client)
    arn = ingestion.role(unique_id, account_id, input_value["collection_arn"])
    return {
        "outcome": "created",
        "role_arn": arn,
        "created": ingestion.created("iam_role"),
        "operations": list(client.scripted.operations),
        "role_name": client.scripted.role_params["RoleName"],
        "policy_name": client.scripted.policy_params["PolicyName"],
        "trust_document": DOCUMENT.validate_json(
            str(client.scripted.role_params["AssumeRolePolicyDocument"])
        ),
        "permissions_document": DOCUMENT.validate_json(
            str(client.scripted.policy_params["PolicyDocument"])
        ),
    }


def flow(input_value: FlowInput) -> dict[str, JsonValue]:
    client = scripted_client()
    install_fakes([], client.scripted.operations)
    client.scripted.statuses = ["ACTIVE"]
    ingestion = build(input_value["vector"], input_value["environment"], client)
    knowledge_base, _file_key = ingestion.flow()
    return {
        "outcome": "initialized",
        "operations": list(client.scripted.operations),
        "knowledge_base_id": knowledge_base,
        "data_source_id": ingestion.data_source_id,
        "s3_bucket": ingestion.s3_bucket,
        "s3_prefix": ingestion.s3_prefix,
        "created_keys": list(ingestion.created_keys()),
        "index_body": INDEX.body,
    }


def store(input_value: StoreInput) -> dict[str, JsonValue]:
    client, ingestion = prepared_ingestion(input_value["vector"], input_value["environment"], [])
    client.scripted.job_statuses = list(input_value["job_statuses"])
    knowledge_base, file_key = ingestion.stored(
        input_value["content"], input_value["filename"], input_value["content_type"]
    )
    return {
        "outcome": "stored",
        "knowledge_base_id": knowledge_base,
        "file_key": file_key,
        "operations": list(client.scripted.operations),
        "content_type": content_type_value(client.scripted.content_type),
    }


def dispatch(input_value: Input) -> JsonValue:
    if input_value["step"] == "policies":
        return policies(input_value)
    if input_value["step"] == "collection":
        return collection(input_value)
    if input_value["step"] == "index":
        return index(input_value)
    if input_value["step"] == "role":
        return role(input_value)
    if input_value["step"] == "flow":
        return flow(input_value)
    return store(input_value)


def run(input_value: Input) -> JsonValue:
    real_sleep = asyncio.sleep
    asyncio.sleep = AsyncMock()
    try:
        return dispatch(input_value)
    finally:
        asyncio.sleep = real_sleep
