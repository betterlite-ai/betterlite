import json
import os
from datetime import UTC, datetime
from functools import partial
from typing import Final, Literal, NotRequired, TypedDict

import botocore.auth  # type: ignore[import-untyped]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock/files/transformation.py#L1125
import fastuuid
import litellm
from fastuuid import UUID
from litellm.llms.bedrock.files.transformation import BedrockFilesConfig, _OpenAIBatchRecord
from pydantic import JsonValue, TypeAdapter

litellm.suppress_debug_info = True

JSON: Final = TypeAdapter(JsonValue)
DEFAULT_ACCESS_KEY: Final = "AKIDEXAMPLE"
DEFAULT_SECRET_KEY: Final = "secret"  # noqa: S105  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock/files/transformation.py#L1125

Purpose = Literal["assistants", "batch", "fine-tune", "messages"]


class FileInput(TypedDict):
    content: str
    filename: str | None
    content_type: str | None


class TransformRecords(TypedDict):
    function: Literal["transform_records"]
    records: list[_OpenAIBatchRecord]
    target_model: str
    model_alias_map: dict[str, str]
    model_cost: dict[str, JsonValue]


class CreateFileRequest(TypedDict):
    function: Literal["create_file_request"]
    model: str
    file: FileInput
    purpose: Purpose | None
    optional_params: dict[str, JsonValue]
    litellm_params: dict[str, JsonValue]
    environment: dict[str, str]
    model_alias_map: dict[str, str]
    model_cost: dict[str, JsonValue]
    uuid: str
    now: int


Input = TransformRecords | CreateFileRequest


class Handoff(TypedDict):
    method: str
    url: str
    headers: dict[str, str]
    data: str


class CreateFileData(TypedDict):
    file: tuple[str | None, bytes, str | None]
    purpose: NotRequired[Purpose]


class FixedNow:
    def __init__(self, seconds: int) -> None:
        self.moment = datetime.fromtimestamp(seconds, tz=UTC).replace(tzinfo=None)

    def __call__(self) -> datetime:
        return self.moment


# The owned entries are private, so the subclass reaches them through self.
class BatchRecordsConfig(BedrockFilesConfig):
    def converted(self, records: list[_OpenAIBatchRecord], target_model: str) -> JsonValue:
        transformed = self._transform_openai_jsonl_content_to_bedrock_jsonl_content(
            records, target_model=target_model
        )
        answer: dict[str, JsonValue] = {"records": JSON.validate_json(json.dumps(transformed))}
        return JSON.validate_json(json.dumps(answer))

    def upload_request(self, call: CreateFileRequest, data: CreateFileData) -> JsonValue:
        params = dict(call["litellm_params"])
        handoff = self.transform_create_file_request(  # type: ignore[misc]  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock/files/transformation.py#L1026
            model=call["model"],
            create_file_data=data,  # type: ignore[arg-type]  # pyright: ignore[reportArgumentType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock/files/transformation.py#L1029
            optional_params=call["optional_params"],
            litellm_params=params,
        )
        view = TypeAdapter(Handoff).validate_json(json.dumps(handoff))  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock/files/transformation.py#L1026
        answer: dict[str, JsonValue] = {
            "failed": False,
            "method": view["method"],
            "url": view["url"],
            "headers": JSON.validate_json(json.dumps(view["headers"])),
            "data": view["data"],
            "litellm_params": JSON.validate_json(json.dumps(params)),
        }
        return JSON.validate_json(json.dumps(answer))


def reset_environment(environment: dict[str, str]) -> None:
    os.environ.clear()
    os.environ.update(
        {
            "LITELLM_RUST": "false",
            "AWS_CONFIG_FILE": os.devnull,
            "AWS_SHARED_CREDENTIALS_FILE": os.devnull,
            "AWS_ACCESS_KEY_ID": DEFAULT_ACCESS_KEY,
            "AWS_SECRET_ACCESS_KEY": DEFAULT_SECRET_KEY,
        }
    )
    os.environ.update(environment)


def upload(config: BatchRecordsConfig, call: CreateFileRequest) -> JsonValue:
    data: CreateFileData = {
        "file": (
            call["file"]["filename"],
            call["file"]["content"].encode(),
            call["file"]["content_type"],
        )
    }
    if call["purpose"] is not None:
        data["purpose"] = call["purpose"]
    fastuuid.uuid4 = partial(UUID, call["uuid"])  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock/files/transformation.py#L477
    try:
        return config.upload_request(call, data)
    except (ValueError, RuntimeError):
        return {"failed": True}


def run(input_value: Input) -> JsonValue:
    if input_value["function"] == "transform_records":
        environment: dict[str, str] = {}
    else:
        environment = input_value["environment"]
    reset_environment(environment)
    litellm.model_alias_map = input_value["model_alias_map"]
    litellm.model_cost = input_value["model_cost"]
    if input_value["function"] == "transform_records":
        config = BatchRecordsConfig()  # type: ignore[no-untyped-call]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock/files/transformation.py#L392-L394
        return config.converted(input_value["records"], input_value["target_model"])
    config = BatchRecordsConfig()  # type: ignore[no-untyped-call]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock/files/transformation.py#L392-L394
    botocore.auth.get_current_datetime = FixedNow(input_value["now"])  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock/files/transformation.py#L1172
    return upload(config, input_value)
