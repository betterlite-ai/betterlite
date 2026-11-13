import os
import time
from collections.abc import Callable
from datetime import UTC, datetime
from functools import partial
from typing import Final, Literal, TypeAlias, TypedDict, TypeVar

import httpx
import litellm
from litellm.litellm_core_utils.litellm_logging import Logging
from litellm.llms.base_llm.chat.transformation import BaseLLMException
from litellm.llms.xai.files.transformation import XAIFilesConfig
from pydantic import JsonValue, TypeAdapter

StepT = TypeVar("StepT")
CONFIG: Final = XAIFilesConfig.__new__(XAIFilesConfig)
JSON: Final[TypeAdapter[JsonValue]] = TypeAdapter(JsonValue)
LOG: Final = Logging(
    model="",
    messages=None,
    stream=False,
    call_type="",
    start_time=datetime.now(UTC),
    litellm_call_id="",
    function_id="",
)


class Auth(TypedDict):
    step: Literal["auth"]
    api_key: str | None


class Url(TypedDict):
    step: Literal["url"]
    api_base: str | None
    target: Literal["collection", "file", "content"]
    file_id: str | None


class Reply(TypedDict):
    step: Literal["reply"]
    parse: Literal["file", "deleted", "next", "list"]
    api_base: str | None
    now: int
    status: int
    body: str


class Content(TypedDict):
    step: Literal["content"]
    status: int
    headers: list[list[str]]
    body: str


Step: TypeAlias = Auth | Url | Reply | Content


class Input(TypedDict):
    env: dict[str, str]
    xai_key: str | None
    steps: list[Step]


def reply_step(step: Reply) -> JsonValue:
    time.time = partial(float, step["now"])
    response = httpx.Response(step["status"], content=step["body"].encode())
    match step["parse"]:
        case "file":
            created = CONFIG.transform_create_file_response(
                model=None, raw_response=response, logging_obj=LOG, litellm_params={}
            )
            return JSON.validate_json(created.model_dump_json(exclude_none=True))
        case "deleted":
            deleted = CONFIG.transform_delete_file_response(
                raw_response=response, logging_obj=LOG, litellm_params={}
            )
            return JSON.validate_json(deleted.model_dump_json(exclude_none=True))
        case "next":
            page = CONFIG.transform_list_files_next_request(
                raw_response=response,
                optional_params={},
                litellm_params={"api_base": step["api_base"]},
            )
            return (
                None
                if page is None
                else [page[0], [[name, value] for name, value in page[1].items()]]
            )
        case "list":
            files = CONFIG.transform_list_files_response(
                raw_response=response, logging_obj=LOG, litellm_params={}
            )
            return [JSON.validate_json(item.model_dump_json(exclude_none=True)) for item in files]


def url_step(step: Url) -> JsonValue:
    litellm_params = {"api_base": step["api_base"]}
    file_id = step["file_id"]
    match step["target"]:
        case "collection":
            return CONFIG.get_complete_url(
                api_base=step["api_base"],
                api_key=None,
                model="",
                optional_params={},
                litellm_params={},
            )
        case "file":
            url, _query = CONFIG.transform_retrieve_file_request(
                file_id="" if file_id is None else file_id,
                optional_params={},
                litellm_params=litellm_params,
            )
            return url
        case "content":
            url, _query = CONFIG.transform_file_content_request(
                file_content_request={} if file_id is None else {"file_id": file_id},
                optional_params={},
                litellm_params=litellm_params,
            )
            return url


def outcome(step: Step) -> JsonValue:
    match step["step"]:
        case "auth":
            headers = CONFIG.validate_environment(
                headers={},
                model="",
                messages=[],
                optional_params={},
                litellm_params={},
                api_key=step["api_key"],
            )
            return headers.get("Authorization")
        case "url":
            return url_step(step)
        case "reply":
            return reply_step(step)
        case "content":
            given = [(name, value) for name, value in step["headers"]]
            response = httpx.Response(step["status"], content=step["body"].encode(), headers=given)
            content = CONFIG.transform_file_content_response(
                raw_response=response, logging_obj=LOG, litellm_params={}
            )
            watched: dict[str, JsonValue] = {
                name.lower(): content.response.headers[name] for name, _ in given
            }
            return {
                "status": content.response.status_code,
                "headers": watched,
                "body": list(content.response.content),
            }


def guarded(outcome: Callable[[StepT], JsonValue], step: StepT) -> JsonValue:
    try:
        return outcome(step)
    except BaseLLMException as error:  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/base_llm/chat/transformation.py#L42
        return {"raised": error.status_code}
    except ValueError:
        return {"raised": None}


def run(input_value: Input) -> list[JsonValue]:
    for name in ("XAI_API_KEY", "XAI_API_BASE"):
        os.environ.pop(name, None)
    os.environ.update(input_value["env"])
    litellm.xai_key = input_value["xai_key"]
    return [guarded(outcome, step) for step in input_value["steps"]]
