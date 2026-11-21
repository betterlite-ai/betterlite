import json
import os
import time
from datetime import UTC, datetime
from functools import partial
from typing import Final, Literal, NotRequired, TypedDict
from unittest.mock import MagicMock

import botocore.auth  # type: ignore[import-untyped]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock/files/transformation.py#L1125
import fastuuid
import httpx
import litellm
from fastuuid import UUID
from litellm.llms.bedrock.files.transformation import BedrockFilesConfig
from litellm.types.utils import ExtractedFileData
from pydantic import JsonValue, TypeAdapter

litellm.suppress_debug_info = True

JSON: Final = TypeAdapter(JsonValue)
OBJECT_ID: Final = "11111111-2222-4333-8444-555555555555"
SIGNING_SECONDS: Final = 1_704_067_200
# boto3's environment provider hands this pair to LiteLLM when a signing names no keys
ENV_ACCESS_KEY: Final = "AKIDEXAMPLE"
ENV_KEY_VALUE: Final = "secret"

Purpose = Literal["assistants", "batch", "fine-tune", "messages"]


class ObjectFile(TypedDict):
    content: str
    content_hex: NotRequired[str]
    purpose: Purpose
    filename: str | None


class Signing(TypedDict):
    content: str
    api_base: str
    optional_params: dict[str, JsonValue]
    encryption_key_id: str | None


class Response(TypedDict):
    litellm_params: dict[str, JsonValue]
    headers: list[tuple[str, str]]
    now: int


class ErrorCall(TypedDict):
    status_code: int
    message: str
    headers: list[tuple[str, str]]


class UploadFile(TypedDict):
    filename: str | None
    content: str


class Upload(TypedDict):
    model: str
    litellm_params: dict[str, JsonValue]
    optional_params: dict[str, JsonValue]
    environment: dict[str, str]
    file: NotRequired[UploadFile]
    purpose: NotRequired[Purpose]


class Handoff(TypedDict):
    headers: dict[str, JsonValue]
    litellm_params: dict[str, JsonValue]


class CreateFileData(TypedDict):
    file: NotRequired[tuple[str | None, bytes]]
    purpose: NotRequired[Purpose]


class ObjectNames(TypedDict):
    operation: Literal["object_name"]
    files: list[ObjectFile]


class UrlConversions(TypedDict):
    operation: Literal["url_conversion"]
    urls: list[str]


class Signings(TypedDict):
    operation: Literal["sign"]
    signings: list[Signing]


class Responses(TypedDict):
    operation: Literal["file_response"]
    responses: list[Response]
    errors: list[ErrorCall]


class Uploads(TypedDict):
    operation: Literal["destination"]
    uploads: list[Upload]


class Contract(TypedDict):
    operation: Literal["contract"]
    handoffs: list[Handoff]
    optional_params: dict[str, JsonValue]


Input = ObjectNames | UrlConversions | Signings | Responses | Uploads | Contract


class FileReply(TypedDict):
    id: str
    bytes: int
    created_at: int
    filename: str
    object: str
    purpose: str
    status: str | None


class FixedNow:
    def __init__(self, seconds: int) -> None:
        self.moment = datetime.fromtimestamp(seconds, tz=UTC).replace(tzinfo=None)

    def __call__(self) -> datetime:
        return self.moment


# _sign_s3_request is private and only B646's transform_create_file_request reaches it publicly
class SigningConfig(BedrockFilesConfig):
    def signed(self, signing: Signing) -> JsonValue:
        content = signing["content"]
        base = signing["api_base"]
        params = signing["optional_params"]
        key = signing["encryption_key_id"]
        pair: tuple[dict[str, str], str] = (  # pyright: ignore[reportUnknownVariableType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock/files/transformation.py#L1116
            self._sign_s3_request(  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock/files/transformation.py#L1114
                content, base, params, key
            )
        )
        headers = JSON.validate_json(json.dumps(list(pair[0].items())))  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock/files/transformation.py#L1116
        return {"failed": False, "headers": headers, "body": pair[1]}


def reset_environment(environment: dict[str, str]) -> None:
    os.environ.clear()
    os.environ.update(
        {
            "LITELLM_RUST": "false",
            "AWS_CONFIG_FILE": os.devnull,
            "AWS_SHARED_CREDENTIALS_FILE": os.devnull,
            "AWS_ACCESS_KEY_ID": ENV_ACCESS_KEY,
            "AWS_SECRET_ACCESS_KEY": ENV_KEY_VALUE,
        }
    )
    os.environ.update(environment)
    fastuuid.uuid4 = partial(UUID, OBJECT_ID)


def object_name(config: BedrockFilesConfig, file: ObjectFile) -> JsonValue:
    content = file["content"].encode()
    if "content_hex" in file:
        content = bytes.fromhex(file["content_hex"])
    extracted = ExtractedFileData(
        filename=file["filename"],
        content=content,
        content_type="application/octet-stream",
        headers={},
    )
    try:
        name = config.get_object_name(extracted, file["purpose"])
    except (ValueError, AttributeError):
        return {"failed": True}
    return {"failed": False, "name": name}


def created(config: BedrockFilesConfig, response: Response) -> JsonValue:
    pairs = [(name.encode(), value.encode("latin-1")) for name, value in response["headers"]]
    raw_response = httpx.Response(status_code=200, headers=pairs)
    params = response["litellm_params"]
    saved = time.time
    time.time = partial(float, response["now"])
    try:
        file_object = (
            config
            .transform_create_file_response(  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock/files/transformation.py#L1242
                None, raw_response, MagicMock(), params
            )
        )
    except ValueError:
        return {"failed": True}
    finally:
        time.time = saved
    reply = TypeAdapter(FileReply).validate_json(file_object.model_dump_json())
    return {"failed": False, "file": JSON.validate_json(json.dumps(reply))}


def converted(config: BedrockFilesConfig, url: str) -> JsonValue:
    params: dict[str, JsonValue] = {"upload_url": url, "_s3_upload_content_length": 0}
    return created(config, {"litellm_params": params, "headers": [], "now": 0})


def error_class(config: BedrockFilesConfig, call: ErrorCall) -> JsonValue:
    headers = httpx.Headers(call["headers"])
    error = config.get_error_class(  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock/files/transformation.py#L1265
        call["message"], call["status_code"], headers
    )
    return {"status_code": error.status_code, "message": error.message}


def destination(config: BedrockFilesConfig, upload: Upload) -> JsonValue:
    reset_environment(upload["environment"])
    data: CreateFileData = {}
    if "file" in upload:
        data["file"] = (upload["file"]["filename"], upload["file"]["content"].encode())
    if "purpose" in upload:
        data["purpose"] = upload["purpose"]
    model = upload["model"]
    optional = upload["optional_params"]
    deployment = upload["litellm_params"]
    try:
        url = config.get_complete_file_url(  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock/files/transformation.py#L511
            None, None, model, optional, deployment,
            data,  # type: ignore[arg-type]  # pyright: ignore[reportArgumentType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock/files/transformation.py#L513
        )
    except ValueError:
        return {"failed": True}
    return {"failed": False, "url": url}


def handoff(config: BedrockFilesConfig, call: Handoff) -> JsonValue:
    params = call["litellm_params"]
    unused: dict[str, JsonValue] = {}
    merged: dict[str, JsonValue] = (  # pyright: ignore[reportUnknownVariableType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock/files/transformation.py#L416
        config.validate_environment(  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock/files/transformation.py#L412
            {**call["headers"]},
            "",
            [],  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock/files/transformation.py#L411
            unused,
            params,  # type: ignore[arg-type]  # pyright: ignore[reportArgumentType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock/files/transformation.py#L413
        )
    )
    return {"headers": merged, "litellm_params": params}


def contract(config: BedrockFilesConfig, call: Contract) -> JsonValue:
    handoffs = [handoff(config, item) for item in call["handoffs"]]
    optional = call["optional_params"]
    mapped: dict[str, JsonValue] = (  # pyright: ignore[reportUnknownVariableType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock/files/transformation.py#L558
        config.map_openai_params(  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock/files/transformation.py#L554
            optional, optional, "", drop_params=False
        )
    )
    answer = {
        "custom_llm_provider": config.custom_llm_provider.value,
        "file_upload_http_method": config.file_upload_http_method,
        "supported_openai_params": config.get_supported_openai_params(""),
        "mapped": mapped,
        "handoffs": handoffs,
    }
    return JSON.validate_json(json.dumps(answer))


def signings(call: Signings) -> JsonValue:
    botocore.auth.get_current_datetime = FixedNow(SIGNING_SECONDS)  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock/files/transformation.py#L1172
    config = SigningConfig()  # type: ignore[no-untyped-call]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock/files/transformation.py#L392-L394
    results: list[JsonValue] = []
    for signing in call["signings"]:
        try:
            results.append(config.signed(signing))
        except ValueError:
            results.append({"failed": True})
    return {"results": results}


def responses(config: BedrockFilesConfig, call: Responses) -> JsonValue:
    files = [created(config, response) for response in call["responses"]]
    errors = [error_class(config, error) for error in call["errors"]]
    return {"results": files, "errors": errors}


def run(input_value: Input) -> JsonValue:
    reset_environment({})
    config = BedrockFilesConfig()  # type: ignore[no-untyped-call]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock/files/transformation.py#L392-L394
    if input_value["operation"] == "object_name":
        return {"results": [object_name(config, file) for file in input_value["files"]]}
    if input_value["operation"] == "url_conversion":
        return {"results": [converted(config, url) for url in input_value["urls"]]}
    if input_value["operation"] == "destination":
        return {"results": [destination(config, upload) for upload in input_value["uploads"]]}
    if input_value["operation"] == "contract":
        return contract(config, input_value)
    if input_value["operation"] == "file_response":
        return responses(config, input_value)
    return signings(input_value)
