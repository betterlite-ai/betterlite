import json
import os
import time
import xml.etree.ElementTree as ET
from datetime import UTC, datetime
from types import MappingProxyType
from typing import Final, Literal, NotRequired, TypedDict, cast
from unittest.mock import MagicMock

import botocore.auth  # type: ignore[import-untyped]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock/files/transformation.py#L1459-L1488
import httpx
import litellm
from litellm.llms.bedrock.common_utils import BedrockError
from litellm.llms.bedrock.files.transformation import BedrockFilesConfig
from litellm.types.llms.openai import FileContentRequest
from pydantic import JsonValue, TypeAdapter

litellm.suppress_debug_info = True

JSON: Final = TypeAdapter(JsonValue)
SIGNING_SECONDS: Final = 1_704_067_200
ENV_ACCESS_KEY: Final = "AKIDEXAMPLE"
ENV_KEY_VALUE: Final = "secret"
TRUSTED_KEY: Final = "_litellm_internal_model_credentials"
BASE_ENVIRONMENT: Final[dict[str, str]] = {
    "LITELLM_RUST": "false",
    "AWS_CONFIG_FILE": os.devnull,
    "AWS_SHARED_CREDENTIALS_FILE": os.devnull,
    "AWS_ACCESS_KEY_ID": ENV_ACCESS_KEY,
    "AWS_SECRET_ACCESS_KEY": ENV_KEY_VALUE,
    "TZ": "UTC",
}


class ListCall(TypedDict):
    purpose: NotRequired[str | None]
    litellm_params: NotRequired[dict[str, JsonValue]]
    optional_params: NotRequired[dict[str, JsonValue]]
    environment: NotRequired[dict[str, str]]
    trusted: NotRequired[list[tuple[str, JsonValue]]]


class NextCall(TypedDict):
    status: int
    body: str
    headers: NotRequired[list[tuple[str, str]]]
    litellm_params: NotRequired[dict[str, JsonValue]]
    optional_params: NotRequired[dict[str, JsonValue]]
    environment: NotRequired[dict[str, str]]
    trusted: NotRequired[list[tuple[str, JsonValue]]]


class ListingCall(TypedDict):
    status: int
    body: str
    headers: NotRequired[list[tuple[str, str]]]
    litellm_params: NotRequired[dict[str, JsonValue]]
    environment: NotRequired[dict[str, str]]
    trusted: NotRequired[list[tuple[str, JsonValue]]]


class FileCall(TypedDict):
    file_id: NotRequired[str | None]
    litellm_params: NotRequired[dict[str, JsonValue]]
    optional_params: NotRequired[dict[str, JsonValue]]
    environment: NotRequired[dict[str, str]]
    trusted: NotRequired[list[tuple[str, JsonValue]]]


class DeleteResponseCall(TypedDict):
    status: int
    body: str
    headers: NotRequired[list[tuple[str, str]]]
    additional_args: NotRequired[dict[str, JsonValue] | None]


class RetrieveCall(TypedDict):
    file_id: str
    litellm_params: NotRequired[dict[str, JsonValue]]
    optional_params: NotRequired[dict[str, JsonValue]]


class ListRequests(TypedDict):
    operation: Literal["list_request"]
    requests: list[ListCall]


class NextRequests(TypedDict):
    operation: Literal["next_request"]
    next: list[NextCall]


class ListResponses(TypedDict):
    operation: Literal["list_response"]
    listings: list[ListingCall]


class DeleteRequests(TypedDict):
    operation: Literal["delete_request"]
    deletes: list[FileCall]


class ContentRequests(TypedDict):
    operation: Literal["content_request"]
    contents: list[FileCall]


class DeleteResponses(TypedDict):
    operation: Literal["delete_response"]
    delete_responses: list[DeleteResponseCall]


class Retrieves(TypedDict):
    operation: Literal["retrieve"]
    retrieves: list[RetrieveCall]


Input = (
    ListRequests
    | NextRequests
    | ListResponses
    | DeleteRequests
    | ContentRequests
    | DeleteResponses
    | Retrieves
)


class FileReply(TypedDict):
    id: str
    bytes: int
    created_at: int
    filename: str
    object: str
    purpose: str
    status: str | None


class FileDeletedReply(TypedDict):
    id: str
    object: str
    deleted: bool


class FixedNow:
    def __init__(self, seconds: int) -> None:
        self.moment = datetime.fromtimestamp(seconds, tz=UTC).replace(tzinfo=None)

    def __call__(self) -> datetime:
        return self.moment


def reset_environment(environment: dict[str, str] | None) -> None:
    os.environ.clear()
    os.environ.update(BASE_ENVIRONMENT)
    if environment is not None:
        os.environ.update(environment)
    time.tzset()


def params_of(
    declared: dict[str, JsonValue] | None, trusted: list[tuple[str, JsonValue]] | None
) -> dict[str, JsonValue]:
    params: dict[str, JsonValue] = {} if declared is None else dict(declared)
    if trusted is not None:
        params[TRUSTED_KEY] = cast(JsonValue, MappingProxyType(dict(trusted)))  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock/files/transformation.py#L192-L198
    return params


def optional_of(declared: dict[str, JsonValue] | None) -> dict[str, JsonValue]:
    if declared is None:
        return {}
    return dict(declared)


def visible(params: dict[str, JsonValue]) -> JsonValue:
    without_trusted = {name: value for name, value in params.items() if name != TRUSTED_KEY}
    return JSON.validate_json(json.dumps(without_trusted, default=dict))


def reply(answer: JsonValue) -> JsonValue:
    encoded: Final = json.dumps(answer)
    return JSON.validate_json(encoded)


def response_of(
    status: int, body: str, headers: list[tuple[str, str]] | None
) -> httpx.Response:
    pairs: list[tuple[str, str]] = [] if headers is None else headers
    return httpx.Response(status_code=status, headers=pairs, content=body.encode())


def list_request(config: BedrockFilesConfig, call: ListCall) -> JsonValue:
    reset_environment(call.get("environment"))
    params = params_of(call.get("litellm_params"), call.get("trusted"))
    optional = optional_of(call.get("optional_params"))
    try:
        url, query = config.transform_list_files_request(  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock/files/transformation.py#L1309-L1317
            call.get("purpose"),
            optional,
            params,  # type: ignore[arg-type]  # pyright: ignore[reportArgumentType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock/files/transformation.py#L1309-L1317
        )
    except BedrockError as error:
        return reply({"failed": True, "status": error.status_code})
    except ValueError:
        return reply({"failed": True, "status": None})
    query_value: Final = JSON.validate_json(json.dumps(query))
    return reply(
        {
            "failed": False,
            "url": url,
            "query": query_value,
            "litellm_params": visible(params),
        }
    )


def next_request(config: BedrockFilesConfig, call: NextCall) -> JsonValue:
    reset_environment(call.get("environment"))
    params = params_of(call.get("litellm_params"), call.get("trusted"))
    optional = optional_of(call.get("optional_params"))
    response = response_of(call["status"], call["body"], call.get("headers"))
    try:
        signed = config.transform_list_files_next_request(  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock/files/transformation.py#L1319-L1335
            response,
            optional,
            params,  # type: ignore[arg-type]  # pyright: ignore[reportArgumentType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock/files/transformation.py#L1319-L1335
        )
    except BedrockError as error:
        return reply({"failed": True, "status": error.status_code})
    except (ValueError, ET.ParseError):
        return reply({"failed": True, "status": None})
    if signed is None:
        return reply({"failed": False, "next": None})
    url, query = signed
    query_value: Final = JSON.validate_json(json.dumps(query))
    next_value: dict[str, JsonValue] = {
        "url": url,
        "query": query_value,
        "litellm_params": visible(params),
    }
    return reply({"failed": False, "next": next_value})


def list_response(config: BedrockFilesConfig, call: ListingCall) -> JsonValue:
    reset_environment(call.get("environment"))
    params = params_of(call.get("litellm_params"), call.get("trusted"))
    response = response_of(call["status"], call["body"], call.get("headers"))
    try:
        files = config.transform_list_files_response(  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock/files/transformation.py#L1363-L1391
            response, MagicMock(), params
        )
    except BedrockError as error:
        return reply({"failed": True, "status": error.status_code})
    except (ValueError, ET.ParseError):
        return reply({"failed": True, "status": None})
    listed: Final = [TypeAdapter(FileReply).validate_json(item.model_dump_json()) for item in files]
    return reply({"failed": False, "files": JSON.validate_json(json.dumps(listed))})


def delete_request(config: BedrockFilesConfig, call: FileCall) -> JsonValue:
    reset_environment(call.get("environment"))
    params = params_of(call.get("litellm_params"), call.get("trusted"))
    optional = optional_of(call.get("optional_params"))
    declared = call.get("file_id")
    file_id = "" if declared is None else declared
    try:
        url, query = config.transform_delete_file_request(  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock/files/transformation.py#L1284-L1292
            file_id,
            optional,
            params,  # type: ignore[arg-type]  # pyright: ignore[reportArgumentType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock/files/transformation.py#L1284-L1292
        )
    except BedrockError as error:
        return reply({"failed": True, "status": error.status_code})
    except ValueError:
        return reply({"failed": True, "status": None})
    query_value: Final = JSON.validate_json(json.dumps(query))
    return reply(
        {
            "failed": False,
            "url": url,
            "query": query_value,
            "litellm_params": visible(params),
        }
    )


def content_request(config: BedrockFilesConfig, call: FileCall) -> JsonValue:
    reset_environment(call.get("environment"))
    params = params_of(call.get("litellm_params"), call.get("trusted"))
    optional = optional_of(call.get("optional_params"))
    request: FileContentRequest = {}
    declared = call.get("file_id")
    if declared is not None:
        request["file_id"] = declared
    try:
        url, query = config.transform_file_content_request(  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock/files/transformation.py#L1393-L1412
            request,
            optional,
            params,  # type: ignore[arg-type]  # pyright: ignore[reportArgumentType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock/files/transformation.py#L1393-L1412
        )
    except BedrockError as error:
        return reply({"failed": True, "status": error.status_code})
    except ValueError:
        return reply({"failed": True, "status": None})
    query_value: Final = JSON.validate_json(json.dumps(query))
    return reply(
        {
            "failed": False,
            "url": url,
            "query": query_value,
            "litellm_params": visible(params),
        }
    )


def delete_response(config: BedrockFilesConfig, call: DeleteResponseCall) -> JsonValue:
    response = response_of(call["status"], call["body"], call.get("headers"))
    declared = call.get("additional_args")
    details: dict[str, JsonValue] = {}
    if declared is not None:
        details["additional_args"] = declared
    logging_obj = MagicMock()
    logging_obj.model_call_details = details
    try:
        deleted = config.transform_delete_file_response(  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock/files/transformation.py#L1294-L1307
            response, logging_obj, {}
        )
    except BedrockError as error:
        return reply({"failed": True, "status": error.status_code})
    except ValueError:
        return reply({"failed": True, "status": None})
    deleted_reply: Final = TypeAdapter(FileDeletedReply).validate_json(deleted.model_dump_json())
    return reply({"failed": False, "file": JSON.validate_json(json.dumps(deleted_reply))})


def retrieve(config: BedrockFilesConfig, call: RetrieveCall) -> list[JsonValue]:
    params: dict[str, JsonValue] = {}
    optional: dict[str, JsonValue] = {}
    response = httpx.Response(status_code=200, content=b"")
    outcomes: list[JsonValue] = []
    try:
        _request = config.transform_retrieve_file_request(  # pyright: ignore[reportUnknownMemberType, reportUnknownVariableType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock/files/transformation.py#L1268-L1274
            call["file_id"], optional, params
        )
    except NotImplementedError:
        outcomes.append(reply({"failed": True, "status": None}))
    try:
        _response = config.transform_retrieve_file_response(  # pyright: ignore[reportUnknownMemberType, reportUnknownVariableType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock/files/transformation.py#L1276-L1282
            response, MagicMock(), params
        )
    except NotImplementedError:
        outcomes.append(reply({"failed": True, "status": None}))
    return outcomes


def run(input_value: Input) -> JsonValue:
    botocore.auth.get_current_datetime = FixedNow(SIGNING_SECONDS)  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock/files/transformation.py#L1459-L1488
    config = BedrockFilesConfig()  # type: ignore[no-untyped-call]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock/files/transformation.py#L392-L394
    if input_value["operation"] == "list_request":
        return reply(
            {"results": [list_request(config, call) for call in input_value["requests"]]}
        )
    if input_value["operation"] == "next_request":
        return reply({"results": [next_request(config, call) for call in input_value["next"]]})
    if input_value["operation"] == "list_response":
        return reply(
            {"results": [list_response(config, call) for call in input_value["listings"]]}
        )
    if input_value["operation"] == "delete_request":
        return reply(
            {"results": [delete_request(config, call) for call in input_value["deletes"]]}
        )
    if input_value["operation"] == "content_request":
        return reply(
            {"results": [content_request(config, call) for call in input_value["contents"]]}
        )
    if input_value["operation"] == "delete_response":
        return reply(
            {
                "results": [
                    delete_response(config, call) for call in input_value["delete_responses"]
                ]
            }
        )
    return reply(
        {
            "results": [
                entry
                for call in input_value["retrieves"]
                for entry in retrieve(config, call)
            ]
        }
    )
