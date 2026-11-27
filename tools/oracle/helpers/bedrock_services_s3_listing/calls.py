import json
import os
import time
import xml.etree.ElementTree as ET
from datetime import UTC, datetime
from typing import Final

import httpx
from litellm.llms.bedrock.common_utils import BedrockError
from litellm.llms.bedrock.files.transformation import BedrockFilesConfig
from litellm.types.llms.openai import FileContentRequest
from pydantic import JsonValue, TypeAdapter

from .schema import Call, RequestOperation

JSON: Final = TypeAdapter(JsonValue)
SIGNING_SECONDS: Final = 1_704_067_200
TRUSTED_KEY: Final = "_litellm_internal_model_credentials"
BASE_ENVIRONMENT: Final[dict[str, str]] = {
    "LITELLM_RUST": "false",
    "AWS_CONFIG_FILE": os.devnull,
    "AWS_SHARED_CREDENTIALS_FILE": os.devnull,
    "AWS_ACCESS_KEY_ID": "AKIDEXAMPLE",
    "AWS_SECRET_ACCESS_KEY": "secret",
    "TZ": "UTC",
}


class FixedNow:
    def __init__(self, seconds: int) -> None:
        self.moment = datetime.fromtimestamp(seconds, tz=UTC).replace(tzinfo=None)

    def __call__(self) -> datetime:
        return self.moment


def reply(answer: JsonValue) -> JsonValue:
    encoded: Final = json.dumps(answer)
    return JSON.validate_json(encoded)


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
        params[TRUSTED_KEY] = dict(trusted)
    return params


def optional_of(declared: dict[str, JsonValue] | None) -> dict[str, JsonValue]:
    if declared is None:
        return {}
    return dict(declared)


def visible(params: dict[str, JsonValue]) -> JsonValue:
    without_trusted = {name: value for name, value in params.items() if name != TRUSTED_KEY}
    return JSON.validate_json(json.dumps(without_trusted))


def response_of(status: int, body: str, headers: list[tuple[str, str]] | None) -> httpx.Response:
    pairs: list[tuple[str, str]] = [] if headers is None else headers
    return httpx.Response(status_code=status, headers=pairs, content=body.encode())


def request_reply(params: dict[str, JsonValue], transformed: tuple[str, dict[str, str]]) -> JsonValue:
    url, query = transformed
    body: dict[str, JsonValue] = {
        "failed": False,
        "url": url,
        "query": JSON.validate_json(json.dumps(query)),
        "litellm_params": visible(params),
    }
    return reply(body)


def signed_request(config: BedrockFilesConfig, operation: RequestOperation, call: Call) -> JsonValue:
    reset_environment(call.get("environment"))
    params = params_of(call.get("litellm_params"), call.get("trusted"))
    optional = optional_of(call.get("optional_params"))
    next_body: dict[str, JsonValue] | None = None
    try:
        if operation == "next_request":
            status = call.get("status")
            body = call.get("body")
            response = response_of(
                0 if status is None else status, "" if body is None else body, call.get("headers")
            )
            signed = config.transform_list_files_next_request(  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock/files/transformation.py#L1319-L1335
                response,
                optional,
                params,  # type: ignore[arg-type]  # pyright: ignore[reportArgumentType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock/files/transformation.py#L1319-L1335
            )
            if signed is not None:
                url, query = signed
                next_body = {
                    "url": url,
                    "query": JSON.validate_json(json.dumps(query)),
                    "litellm_params": visible(params),
                }
            result = reply({"failed": False, "next": next_body})
        elif operation == "delete_request":
            declared = call.get("file_id")
            file_id = "" if declared is None else declared
            result = request_reply(
                params,
                config.transform_delete_file_request(  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock/files/transformation.py#L1284-L1292
                    file_id,
                    optional,
                    params,  # type: ignore[arg-type]  # pyright: ignore[reportArgumentType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock/files/transformation.py#L1284-L1292
                ),
            )
        elif operation == "content_request":
            content: FileContentRequest = {}
            declared = call.get("file_id")
            if declared is not None:
                content["file_id"] = declared
            result = request_reply(
                params,
                config.transform_file_content_request(  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock/files/transformation.py#L1393-L1412
                    content,
                    optional,
                    params,  # type: ignore[arg-type]  # pyright: ignore[reportArgumentType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock/files/transformation.py#L1393-L1412
                ),
            )
        else:
            result = request_reply(
                params,
                config.transform_list_files_request(  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock/files/transformation.py#L1309-L1317
                    call.get("purpose"),
                    optional,
                    params,  # type: ignore[arg-type]  # pyright: ignore[reportArgumentType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock/files/transformation.py#L1309-L1317
                ),
            )
    except BedrockError as error:
        return reply({"failed": True, "status": error.status_code})
    except (ValueError, ET.ParseError):
        return reply({"failed": True, "status": None})
    return result
