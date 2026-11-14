import os
from datetime import UTC, datetime
from types import SimpleNamespace
from typing import Literal, NotRequired, Protocol, TypedDict
from unittest.mock import patch
from uuid import UUID

import httpx
import litellm
import litellm.llms.infinity.rerank.transformation as infinity_module
import litellm.llms.jina_ai.rerank.transformation as jina_module
from litellm.litellm_core_utils.litellm_logging import Logging
from litellm.llms.infinity.rerank.transformation import InfinityRerankConfig
from litellm.llms.jina_ai.rerank.transformation import JinaAIRerankConfig
from litellm.types.rerank import RerankBilledUnits, RerankResponse
from pydantic import JsonValue, TypeAdapter

Params = dict[str, JsonValue]
Document = str | dict[str, JsonValue]
ADAPTER: TypeAdapter[JsonValue] = TypeAdapter(JsonValue)
JINA_MODEL = "jina_ai/oracle-rerank"


class Options(TypedDict):
    query: str
    documents: list[Document]
    top_n: NotRequired[int | None]


class Reply(TypedDict):
    status: int
    body: str


class Input(TypedDict):
    provider: Literal["jina", "infinity"]
    model: str
    options: Options
    non_default: NotRequired[Params]
    headers: NotRequired[dict[str, str]]
    env: NotRequired[dict[str, str]]
    global_key: NotRequired[str | None]
    api_base: NotRequired[str | None]
    api_key: NotRequired[str | None]
    input_cost_per_token: NotRequired[float | None]
    reply: NotRequired[Reply | None]


class RerankConfig(Protocol):
    def map_cohere_rerank_params(
        self,
        non_default_params: Params,
        model: str,
        drop_params: bool,
        query: str,
        documents: list[Document],
        custom_llm_provider: str | None,
        top_n: int | None,
        rank_fields: list[str] | None,
        return_documents: bool | None,
        max_chunks_per_doc: int | None,
        max_tokens_per_doc: int | None,
        instruction: str | None,
    ) -> Params: ...

    def validate_environment(
        self, headers: dict[str, str], model: str, api_key: str | None, optional_params: Params
    ) -> dict[str, str]: ...

    def get_complete_url(
        self, api_base: str | None, model: str, optional_params: Params
    ) -> str: ...

    def transform_rerank_request(
        self, model: str, optional_rerank_params: Params, headers: dict[str, str]
    ) -> Params: ...

    def transform_rerank_response(
        self,
        model: str,
        raw_response: httpx.Response,
        model_response: RerankResponse,
        logging_obj: Logging,
    ) -> RerankResponse: ...


class Hook(list[JsonValue]):
    def __call__(self, details: dict[str, object]) -> None:
        self.append(ADAPTER.validate_python(details["original_response"]))


FIXED_UUID = SimpleNamespace(uuid4=lambda: UUID("00000000-0000-4000-8000-000000000001"))


def jina_cost(call: Input, billed: RerankBilledUnits | None) -> JsonValue:
    litellm.register_model(
        {
            JINA_MODEL: {
                "input_cost_per_token": call.get("input_cost_per_token"),
                "litellm_provider": "jina_ai",
                "mode": "rerank",
            }
        }
    )
    model_info = litellm.get_model_info(model=JINA_MODEL, custom_llm_provider="jina_ai")
    prompt, completion = JinaAIRerankConfig().calculate_rerank_cost(
        call["model"], "jina_ai", billed, model_info
    )
    return [prompt, completion]


def run(input_value: Input) -> JsonValue:
    config: RerankConfig = (
        JinaAIRerankConfig() if input_value["provider"] == "jina" else InfinityRerankConfig()
    )
    options = input_value["options"]
    model = input_value["model"]
    hook = Hook()
    logging_object = Logging(
        model="rerank-model",
        messages=None,
        stream=False,
        call_type="rerank",
        start_time=datetime(2026, 1, 1, tzinfo=UTC),
        litellm_call_id="rerank-call",
        function_id="rerank-function",
    )
    logging_object.update_environment_variables(
        litellm_params={"logger_fn": hook}, optional_params={}
    )
    litellm.infinity_key = input_value.get("global_key")
    with (
        patch.dict(os.environ, input_value.get("env", {}), clear=True),
        patch.object(jina_module, "uuid", new=FIXED_UUID),
        patch.object(infinity_module, "uuid", new=FIXED_UUID),
    ):
        mapped = config.map_cohere_rerank_params(
            input_value.get("non_default", {}),
            model,
            False,
            options["query"],
            options["documents"],
            None,
            options.get("top_n"),
            None,
            True,
            None,
            None,
            None,
        )
        headers = config.validate_environment(
            input_value.get("headers", {}), model, input_value.get("api_key"), mapped
        )
        url = config.get_complete_url(input_value.get("api_base"), model, mapped)
        request = config.transform_rerank_request(model, mapped, headers)
        reply = input_value.get("reply")
        response: JsonValue = None
        cost: JsonValue = None
        if reply is not None:
            transformed = config.transform_rerank_response(
                model,
                httpx.Response(reply["status"], content=reply["body"]),
                RerankResponse(),
                logging_object,
            )
            response = ADAPTER.validate_json(transformed.model_dump_json())
            meta = transformed.meta
            if input_value["provider"] == "jina":
                cost = jina_cost(input_value, None if meta is None else meta.get("billed_units"))
    return {
        "mapped": mapped,
        "headers": {**headers},
        "url": url,
        "request": request,
        "response": response,
        "hook": hook,
        "cost": cost,
    }
