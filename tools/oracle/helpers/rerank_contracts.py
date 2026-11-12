import datetime as dt
from collections.abc import Callable
from typing import Literal, Protocol, Required, TypeAlias, TypedDict, cast

import httpx
from litellm.litellm_core_utils.litellm_logging import Logging
from litellm.llms.base_llm.chat.transformation import BaseLLMException
from litellm.llms.base_llm.rerank.transformation import BaseRerankConfig
from litellm.llms.cohere.rerank.transformation import CohereRerankConfig
from litellm.rerank_api.rerank_utils import get_optional_rerank_params  # pyright: ignore[reportUnknownVariableType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/rerank_api/rerank_utils.py#L20-L52
from litellm.types.rerank import RerankBilledUnits, RerankRequest, RerankResponse
from pydantic import JsonValue, TypeAdapter
from typing_extensions import override

Json: TypeAlias = JsonValue
Document: TypeAlias = str | dict[str, Json]
ADAPTER: TypeAdapter[JsonValue] = TypeAdapter(JsonValue)
ABSENT: Json = ["__absent__"]


class WireParams(TypedDict):
    requests: list[Json]
    responses: list[Json]
    accessor_keys: list[str]


class ProjectionCall(TypedDict, total=False):
    model: Required[str]
    drop_params: Required[bool]
    query: Required[str]
    documents: Required[list[Document]]
    custom_llm_provider: str | None
    top_n: int | None
    rank_fields: list[str] | None
    return_documents: bool | None
    max_chunks_per_doc: int | None
    max_tokens_per_doc: int | None
    instruction: str | None
    nondefault: dict[str, Json]


ProjectionParams: TypeAlias = dict[str, list[ProjectionCall]]


class ErrorParams(TypedDict):
    message: str
    status_code: int
    headers: dict[str, Json]


class DefaultsParams(TypedDict):
    api_bases: list[str | None]
    response: Json
    error: ErrorParams


class CostCase(TypedDict, total=False):
    model_info: dict[str, Json]
    billed_units: RerankBilledUnits


class CostParams(TypedDict):
    model: str
    cases: list[CostCase]


class Input(TypedDict):
    operation: Literal["wire_records", "option_projection", "base_defaults", "query_cost"]
    params: Json


class ResponseGet(Protocol):
    def __call__(self, response: RerankResponse, key: str, default: Json) -> Json: ...


RESPONSE_GET: ResponseGet = cast(  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/rerank.py#L91-L92
    ResponseGet,
    RerankResponse.get,  # type: ignore[misc]  # pyright: ignore[reportAny]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/rerank.py#L91-L92
)


class Recording(CohereRerankConfig):
    def __init__(self) -> None:
        self.calls: list[dict[str, Json]] = []

    @override
    def map_cohere_rerank_params(
        self,
        non_default_params: dict[str, Json] | None,
        model: str,
        drop_params: bool,
        query: str,
        documents: list[Document],
        custom_llm_provider: str | None = None,
        top_n: int | None = None,
        rank_fields: list[str] | None = None,
        return_documents: bool | None = True,
        max_chunks_per_doc: int | None = None,
        max_tokens_per_doc: int | None = None,
        instruction: str | None = None,
    ) -> dict[str, Json]:
        observed = {} if non_default_params is None else dict(non_default_params)
        self.calls.append(
            {
                "non_default_params": observed,
                "model": model,
                "drop_params": drop_params,
                "query": query,
                "documents": ADAPTER.validate_python(documents),
                "custom_llm_provider": custom_llm_provider,
                "top_n": top_n,
                "rank_fields": ADAPTER.validate_python(rank_fields),
                "return_documents": return_documents,
                "max_chunks_per_doc": max_chunks_per_doc,
                "max_tokens_per_doc": max_tokens_per_doc,
                "instruction": instruction,
            }
        )
        return {"mapped": True}


def accessor(response: RerankResponse, key: str) -> Json:
    present = key in response
    item: Json
    try:
        item = {"ok": True, "value": ADAPTER.validate_python(response[key])}  # type: ignore[misc]  # pyright: ignore[reportAny]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/rerank.py#L88-L89
    except KeyError:
        item = {"ok": False}
    resolved: Json = RESPONSE_GET(response, key, ABSENT)
    return {"contains": present, "item": item, "get": resolved}


def request_outcome(payload: Json) -> Json:
    try:
        request = RerankRequest.model_validate(payload)  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/rerank.py#L13-L28
    except ValueError:
        return {"ok": False}
    return {"ok": True, "dump": ADAPTER.validate_json(request.model_dump_json())}


def response_outcome(payload: Json, keys: list[str]) -> Json:
    try:
        response = RerankResponse.model_validate(payload)  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/rerank.py#L80-L95
    except ValueError:
        return {"ok": False}
    accessors: dict[str, Json] = {key: accessor(response, key) for key in keys}
    dump = ADAPTER.validate_json(response.model_dump_json())
    return {"ok": True, "dump": dump, "accessors": accessors}


def wire_records(raw: Json) -> Json:
    params = TypeAdapter(WireParams).validate_python(raw)
    keys = params["accessor_keys"]
    responses = [response_outcome(payload, keys) for payload in params["responses"]]
    return {
        "requests": [request_outcome(payload) for payload in params["requests"]],
        "responses": responses,
    }


def project(config: Recording, call: ProjectionCall) -> Json:
    nondefault = call.get("nondefault")
    params: dict[str, Json] | None = None if nondefault is None else dict(nondefault)
    return_documents: bool | None = True
    if "return_documents" in call:
        return_documents = call["return_documents"]
    returned: Json = ADAPTER.validate_python(
        get_optional_rerank_params(  # type: ignore[misc]  # pyright: ignore[reportAny]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/rerank_api/rerank_utils.py#L20-L52
            rerank_provider_config=config,
            model=call["model"],
            drop_params=call["drop_params"],
            query=call["query"],
            documents=call["documents"],
            custom_llm_provider=call.get("custom_llm_provider"),
            top_n=call.get("top_n"),
            rank_fields=call.get("rank_fields"),
            return_documents=return_documents,
            max_chunks_per_doc=call.get("max_chunks_per_doc"),
            max_tokens_per_doc=call.get("max_tokens_per_doc"),
            instruction=call.get("instruction"),
            non_default_params=params,
        )
    )
    return {
        "mapper": config.calls[0],
        "nondefault_after": None if params is None else params,
        "returned": returned,
    }


def option_projection(raw: Json) -> Json:
    params = TypeAdapter(ProjectionParams).validate_python(raw)
    return {"calls": [project(Recording(), call) for call in params["calls"]]}


def error_outcome(error: ErrorParams) -> Json:
    try:
        BaseRerankConfig.get_error_class(  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/base_llm/rerank/transformation.py#L94-L99
            CohereRerankConfig(),
            error_message=error["message"],
            status_code=error["status_code"],
            headers=error["headers"],
        )
    except BaseLLMException as raised:  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/base_llm/rerank/transformation.py#L42-L65
        return ADAPTER.validate_python(  # pyright: ignore[reportAny]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/base_llm/rerank/transformation.py#L42-L65
            {
                "status_code": raised.status_code,
                "message": raised.message,
                "headers": raised.headers,  # type: ignore[misc]  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/base_llm/chat/transformation.py#L60-L65
            }
        )
    raise AssertionError("get_error_class returned instead of raising")


def response_transform(config: CohereRerankConfig, raw: Json) -> Json:
    response = RerankResponse.model_validate(raw)  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/rerank.py
    callback = httpx.Request("POST", "https://docs.litellm.ai/docs")
    logging = Logging(
        model="rerank-model",
        messages=None,
        stream=False,
        call_type="rerank",
        start_time=dt.datetime.now(dt.UTC),
        litellm_call_id="rerank-contracts",
        function_id="rerank-contracts",
    )
    request_data: dict[str, Json] = {}
    optional: dict[str, Json] = {}
    litellm_params: dict[str, Json] = {}
    transformed = BaseRerankConfig.transform_rerank_response(  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/base_llm/rerank/transformation.py#L43-L55
        config,
        model="rerank-model",
        raw_response=httpx.Response(status_code=200, request=callback, content=b"{}"),
        model_response=response,
        logging_obj=logging,
        api_key=None,
        request_data=request_data,
        optional_params=optional,
        litellm_params=litellm_params,
    )
    return ADAPTER.validate_json(transformed.model_dump_json())


def base_defaults(raw: Json) -> Json:
    params = TypeAdapter(DefaultsParams).validate_python(raw)
    config = CohereRerankConfig()
    urls: list[Json] = [
        BaseRerankConfig.get_complete_url(  # pyright: ignore[reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/base_llm/rerank/transformation.py#L57-L71
            config, api_base, "rerank-model", None
        )
        for api_base in params["api_bases"]
    ]
    headers: dict[str, Json] = {}
    optional: dict[str, Json] = {}
    request: Json = ADAPTER.validate_python(
        BaseRerankConfig.transform_rerank_request(  # type: ignore[misc]  # pyright: ignore[reportUnknownMemberType, reportAny]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/base_llm/rerank/transformation.py#L33-L41
            config, "rerank-model", optional, headers, None
        )
    )
    return {
        "urls": urls,
        "request": request,
        "response": response_transform(config, params["response"]),
        "error": error_outcome(params["error"]),
    }


def query_cost(raw: Json) -> Json:
    params = TypeAdapter(CostParams).validate_python(raw)
    config = CohereRerankConfig()
    costs: list[Json] = []
    for case in params["cases"]:
        prompt, completion = config.calculate_rerank_cost(
            model=params["model"],
            custom_llm_provider=None,
            billed_units=case.get("billed_units"),
            model_info=case.get("model_info"),  # type: ignore[arg-type]  # pyright: ignore[reportArgumentType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/utils.py#L327-L327
        )
        costs.append([prompt, completion])
    return {"costs": costs}


HANDLERS: dict[str, Callable[[Json], Json]] = {
    "wire_records": wire_records,
    "option_projection": option_projection,
    "base_defaults": base_defaults,
    "query_cost": query_cost,
}


def run(input_value: Input) -> Json:
    handler = HANDLERS.get(input_value["operation"])
    if handler is None:
        raise ValueError(f"unknown operation: {input_value['operation']}")
    return handler(input_value["params"])
