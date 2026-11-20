import sys
from dataclasses import dataclass, field
from types import ModuleType

from pydantic import JsonValue, TypeAdapter

COLLECTION_ARN = "arn:aws:aoss:us-west-2:123456789012:collection/abc"
DEFAULT_ENDPOINT = "https://abc.us-west-2.aoss.amazonaws.com"

DOCUMENT: TypeAdapter[JsonValue] = TypeAdapter(JsonValue)


class CollectionCreatedError(Exception):
    pass


class FakeIndexError(Exception):
    pass


@dataclass
class IndexRun:
    errors: list[str] = field(default_factory=list)
    attempts: int = 0
    index_name: str = ""
    body: JsonValue = None
    operations: list[str] = field(default_factory=list)


INDEX: IndexRun = IndexRun()


class FakeIndices:
    def create(self, index: str, body: JsonValue) -> None:
        INDEX.attempts += 1
        INDEX.index_name = index
        INDEX.body = body
        INDEX.operations.append("CreateIndex")
        if INDEX.attempts <= len(INDEX.errors):
            raise FakeIndexError(INDEX.errors[INDEX.attempts - 1])


class FakeOpenSearch:
    def __init__(self, **_params: JsonValue) -> None:
        self.indices = FakeIndices()


class FakeAws4Auth:
    def __init__(self, *args: JsonValue, **params: JsonValue) -> None:
        self.args = args
        self.params = params


class FakeConnection:
    pass


class FakeOpenSearchModule(ModuleType):
    OpenSearch: type[FakeOpenSearch]
    RequestsHttpConnection: type[FakeConnection]


class FakeAws4AuthModule(ModuleType):
    AWS4Auth: type[FakeAws4Auth]


def install_fakes(errors: list[str], operations: list[str]) -> None:
    INDEX.errors = errors
    INDEX.attempts = 0
    INDEX.index_name = ""
    INDEX.body = None
    INDEX.operations = operations
    opensearch = FakeOpenSearchModule("opensearchpy")
    opensearch.OpenSearch = FakeOpenSearch
    opensearch.RequestsHttpConnection = FakeConnection
    sys.modules["opensearchpy"] = opensearch
    aws4auth = FakeAws4AuthModule("requests_aws4auth")
    aws4auth.AWS4Auth = FakeAws4Auth
    sys.modules["requests_aws4auth"] = aws4auth


@dataclass
class Scripted:
    operations: list[str] = field(default_factory=list)
    policies: list[str] = field(default_factory=list)
    statuses: list[str] = field(default_factory=list)
    status_index: int = 0
    role_arn: str = ""
    role_params: dict[str, JsonValue] = field(default_factory=dict)
    policy_params: dict[str, JsonValue] = field(default_factory=dict)
    content_type: str = ""
    job_statuses: list[str] = field(default_factory=list)
    job_index: int = 0
    knowledge_base_id: str = ""
    data_source_id: str = ""
    data_sources: list[JsonValue] = field(default_factory=list)
    data_source: dict[str, JsonValue] = field(default_factory=dict)
    collection_endpoint: str = DEFAULT_ENDPOINT


@dataclass
class FakeClient:
    scripted: Scripted

    def create_security_policy(self, **params: JsonValue) -> dict[str, JsonValue]:
        self.scripted.operations.append("CreateSecurityPolicy")
        self.scripted.policies.append(str(params["policy"]))
        return {}

    def create_access_policy(self, **params: JsonValue) -> dict[str, JsonValue]:
        self.scripted.operations.append("CreateAccessPolicy")
        self.scripted.policies.append(str(params["policy"]))
        return {}

    def create_collection(self, **_params: JsonValue) -> dict[str, JsonValue]:
        self.scripted.operations.append("CreateCollection")
        return {"createCollectionDetail": {"id": "abc", "status": "CREATING"}}

    def batch_get_collection(self, **_params: JsonValue) -> dict[str, JsonValue]:
        self.scripted.operations.append("BatchGetCollection")
        status = self.next_status()
        return {
            "collectionDetails": [
                {
                    "id": "abc",
                    "status": status,
                    "arn": COLLECTION_ARN,
                    "collectionEndpoint": self.scripted.collection_endpoint,
                }
            ]
        }

    def next_status(self) -> str:
        if not self.scripted.statuses:
            return "CREATING"
        index = min(self.scripted.status_index, len(self.scripted.statuses) - 1)
        self.scripted.status_index += 1
        return self.scripted.statuses[index]

    def create_role(self, **params: JsonValue) -> dict[str, JsonValue]:
        self.scripted.operations.append("CreateRole")
        self.scripted.role_params = params
        return {"Role": {"Arn": self.scripted.role_arn}}

    def put_role_policy(self, **params: JsonValue) -> dict[str, JsonValue]:
        self.scripted.operations.append("PutRolePolicy")
        self.scripted.policy_params = params
        return {}

    def put_object(self, **params: JsonValue) -> dict[str, JsonValue]:
        self.scripted.operations.append("PutObject")
        self.scripted.content_type = str(params["ContentType"])
        return {}

    def start_ingestion_job(self, **_params: JsonValue) -> dict[str, JsonValue]:
        self.scripted.operations.append("StartIngestionJob")
        return {"ingestionJob": {"ingestionJobId": "job1", "status": "STARTING"}}

    def get_ingestion_job(self, **_params: JsonValue) -> dict[str, JsonValue]:
        self.scripted.operations.append("GetIngestionJob")
        return {"ingestionJob": {"ingestionJobId": "job1", "status": self.next_job_status()}}

    def next_job_status(self) -> str:
        if not self.scripted.job_statuses:
            return "COMPLETE"
        index = min(self.scripted.job_index, len(self.scripted.job_statuses) - 1)
        self.scripted.job_index += 1
        return self.scripted.job_statuses[index]

    def get_caller_identity(self, **_params: JsonValue) -> dict[str, JsonValue]:
        self.scripted.operations.append("GetCallerIdentity")
        return {
            "Account": "123456789012",
            "Arn": "arn:aws:iam::123456789012:role/caller",
        }

    def create_bucket(self, **_params: JsonValue) -> dict[str, JsonValue]:
        self.scripted.operations.append("CreateBucket")
        return {"Location": "/bucket"}

    def list_data_sources(self, **_params: JsonValue) -> dict[str, JsonValue]:
        self.scripted.operations.append("ListDataSources")
        return {"dataSourceSummaries": self.scripted.data_sources}

    def get_data_source(self, **_params: JsonValue) -> dict[str, JsonValue]:
        self.scripted.operations.append("GetDataSource")
        return self.scripted.data_source

    def create_knowledge_base(self, **_params: JsonValue) -> dict[str, JsonValue]:
        self.scripted.operations.append("CreateKnowledgeBase")
        return {
            "knowledgeBase": {
                "knowledgeBaseId": self.scripted.knowledge_base_id,
                "status": "CREATING",
            }
        }

    def get_knowledge_base(self, **_params: JsonValue) -> dict[str, JsonValue]:
        self.scripted.operations.append("GetKnowledgeBase")
        return {
            "knowledgeBase": {
                "knowledgeBaseId": self.scripted.knowledge_base_id,
                "status": "ACTIVE",
            }
        }

    def create_data_source(self, **_params: JsonValue) -> dict[str, JsonValue]:
        self.scripted.operations.append("CreateDataSource")
        return {"dataSource": {"dataSourceId": self.scripted.data_source_id}}
