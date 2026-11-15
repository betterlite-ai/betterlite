from collections.abc import Sequence
from enum import Enum
from typing import ClassVar, Final, Literal, NotRequired, TypeAlias

from pydantic import BaseModel, ConfigDict, JsonValue
from typing_extensions import ReadOnly, TypedDict


class BedrockS3InputDataConfig(TypedDict):
    s3Uri: str
    s3BucketOwner: NotRequired[ReadOnly[str]]


class BedrockInputDataConfig(TypedDict):
    s3InputDataConfig: BedrockS3InputDataConfig


class BedrockS3OutputDataConfig(TypedDict, total=False):
    s3Uri: str
    s3EncryptionKeyId: str | None
    s3BucketOwner: ReadOnly[str]


class BedrockOutputDataConfig(TypedDict):
    s3OutputDataConfig: BedrockS3OutputDataConfig


class BedrockTag(TypedDict):
    key: str
    value: str


class AwsSessionTag(TypedDict):
    Key: str
    Value: str


class AwsAuthParams(BaseModel):
    model_config: ClassVar[ConfigDict] = ConfigDict(frozen=True, extra="ignore")

    aws_access_key_id: str | None = None
    aws_secret_access_key: str | None = None
    aws_session_token: str | None = None
    aws_session_name: str | None = None
    aws_profile_name: str | None = None
    aws_role_name: str | None = None
    aws_web_identity_token: str | None = None
    aws_sts_endpoint: str | None = None
    aws_external_id: str | None = None
    aws_session_tags: Sequence[AwsSessionTag] | JsonValue = None


AWS_AUTH_PARAM_KEYS: Final[tuple[str, ...]] = tuple(AwsAuthParams.model_fields)


class BedrockCreateBatchRequest(TypedDict, total=False):
    jobName: str
    roleArn: str
    modelId: str
    inputDataConfig: BedrockInputDataConfig
    outputDataConfig: BedrockOutputDataConfig
    timeoutDurationInHours: int | None
    clientRequestToken: str | None
    tags: list[BedrockTag] | None


BedrockBatchJobStatus: TypeAlias = Literal[
    "Submitted", "InProgress", "Completed", "Failed", "Stopping", "Stopped",
]


class BedrockCreateBatchResponse(TypedDict):
    jobArn: str
    jobName: str
    status: BedrockBatchJobStatus


class BedrockGetBatchResponse(TypedDict, total=False):
    jobArn: str
    jobName: str
    modelId: str
    roleArn: str
    status: BedrockBatchJobStatus
    message: str | None
    submitTime: str | None
    lastModifiedTime: str | None
    endTime: str | None
    inputDataConfig: BedrockInputDataConfig
    outputDataConfig: BedrockOutputDataConfig
    timeoutDurationInHours: int | None
    clientRequestToken: str | None


class BedrockBatchRecordKind(Enum):
    CHAT = "chat"
    TEXT_COMPLETION = "text_completion"
    RESPONSES = "responses"
    EMBEDDING = "embedding"
