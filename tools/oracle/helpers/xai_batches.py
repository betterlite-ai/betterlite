import os
from typing import Final, Literal, TypeAlias, TypedDict, cast

import httpx
import litellm
from litellm.llms.custom_httpx.http_handler import HTTPHandler
from litellm.llms.xai.batches.handler import XAIBatchesHandler
from litellm.llms.xai.batches.transformation import (
    DEFAULT_BATCH_ENDPOINT,
    XAIBatch,
    XAIBatchesError,
    XAIBatchList,
    XAIBatchResult,
    get_xai_auth_headers,
    results_to_openai_jsonl,
    to_create_batch_body,
    to_litellm_batch,
    to_openai_batch_list,
    xai_batches_url,
)
from litellm.types.llms.openai import CreateBatchRequest

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]

TIMEOUT: Final = 600.0
EMPTY_LIST_BODY: Final = b'{"batches":[]}'
CLEARED_NAMES: Final = ("XAI_API_KEY", "XAI_API_BASE")


class Script:
    def __init__(self) -> None:
        self.urls: list[str] = []

    def __call__(self, request: httpx.Request) -> httpx.Response:
        self.urls.append(str(request.url))
        return httpx.Response(200, content=EMPTY_LIST_BODY, request=request)


class CreateCall(TypedDict):
    api_base: str | None
    api_key: str | None
    data: CreateBatchRequest
    endpoint: str | None
    batch: dict[str, Json]


class BatchCall(TypedDict):
    batch: dict[str, Json]
    endpoint: str | None


class ListCall(TypedDict):
    page: dict[str, Json]


class JsonlCall(TypedDict):
    results: list[dict[str, Json]]


class ParamsCall(TypedDict):
    api_base: str | None
    api_key: str | None
    after: str | None
    limit: int | None


class Common(TypedDict):
    env: dict[str, str]
    xai_key: str | None


class CreateInput(Common):
    operation: Literal["create"]
    calls: list[CreateCall]


class BatchInput(TypedDict):
    operation: Literal["batch"]
    calls: list[BatchCall]


class ListInput(TypedDict):
    operation: Literal["list"]
    calls: list[ListCall]


class JsonlInput(TypedDict):
    operation: Literal["jsonl"]
    calls: list[JsonlCall]


class ParamsInput(Common):
    operation: Literal["params"]
    calls: list[ParamsCall]


Input: TypeAlias = CreateInput | BatchInput | ListInput | JsonlInput | ParamsInput


def apply_environment(environment: dict[str, str]) -> None:
    for name in CLEARED_NAMES:
        os.environ.pop(name, None)
    os.environ.update(environment)


def create_outcome(call: CreateCall) -> Json:
    try:
        url = xai_batches_url(call["api_base"])
        headers = get_xai_auth_headers({}, api_key=call["api_key"])
        body = to_create_batch_body(call["data"])
        endpoint = DEFAULT_BATCH_ENDPOINT if call["endpoint"] is None else call["endpoint"]
        batch = to_litellm_batch(XAIBatch.model_validate(call["batch"]), endpoint)
    except XAIBatchesError as error:  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/base_llm/chat/transformation.py#L42
        return {"ok": False, "status": error.status_code}
    return {
        "ok": True,
        "url": url,
        "body": {"name": body.get("name"), "input_file_id": body.get("input_file_id")},
        "authorization": headers["Authorization"],
        "batch": cast("dict[str, Json]", batch.model_dump()),  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/utils.py#L4320
    }


def batch_outcome(call: BatchCall) -> Json:
    endpoint = DEFAULT_BATCH_ENDPOINT if call["endpoint"] is None else call["endpoint"]
    batch = to_litellm_batch(XAIBatch.model_validate(call["batch"]), endpoint)
    return {
        "ok": True,
        "batch": cast("dict[str, Json]", batch.model_dump()),  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/utils.py#L4320
    }


def list_outcome(call: ListCall) -> Json:
    page = to_openai_batch_list(XAIBatchList.model_validate(call["page"]))
    return {
        "ok": True,
        "page": cast("dict[str, Json]", page.model_dump()),  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/xai/batches/transformation.py#L211
    }


def jsonl_outcome(call: JsonlCall) -> Json:
    results = [
        XAIBatchResult.model_validate(result) for result in call["results"]
    ]
    return {"ok": True, "jsonl": results_to_openai_jsonl(results).decode()}


def params_outcome(call: ParamsCall) -> Json:
    script = Script()
    client = HTTPHandler(client=httpx.Client(transport=httpx.MockTransport(script)))
    handler = XAIBatchesHandler(sync_client=client)
    try:
        handler.list_batches(
            _is_async=False,
            api_base=call["api_base"],
            api_key=call["api_key"],
            timeout=TIMEOUT,
            after=call["after"],
            limit=call["limit"],
        )
    except XAIBatchesError as error:  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/base_llm/chat/transformation.py#L42
        return {"ok": False, "status": error.status_code}
    return {"ok": True, "url": script.urls[0]}


def run(input_value: Input) -> Json:
    match input_value["operation"]:
        case "jsonl":
            return {"calls": [jsonl_outcome(call) for call in input_value["calls"]]}
        case "create":
            apply_environment(input_value["env"])
            litellm.xai_key = input_value["xai_key"]
            return {"calls": [create_outcome(call) for call in input_value["calls"]]}
        case "batch":
            return {"calls": [batch_outcome(call) for call in input_value["calls"]]}
        case "list":
            return {"calls": [list_outcome(call) for call in input_value["calls"]]}
        case "params":
            apply_environment(input_value["env"])
            litellm.xai_key = input_value["xai_key"]
            return {"calls": [params_outcome(call) for call in input_value["calls"]]}
