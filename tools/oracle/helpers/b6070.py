from collections.abc import Callable
from datetime import UTC, datetime
from decimal import Decimal
from typing import Protocol, TypeAlias, TypedDict, cast

import httpx
import litellm
from litellm.litellm_core_utils.litellm_logging import Logging
from litellm.llms.fireworks_ai import common_utils, cost_calculator
from litellm.llms.fireworks_ai.common_utils import FireworksAIMixin
from litellm.llms.fireworks_ai.cost_calculator import get_base_model_for_pricing
from litellm.llms.fireworks_ai.embed.fireworks_ai_transformation import FireworksAIEmbeddingConfig
from litellm.llms.fireworks_ai.rerank.transformation import FireworksAIRerankConfig
from litellm.types.rerank import RerankResponse
from litellm.types.utils import Usage
from pydantic import JsonValue, TypeAdapter

JSON: TypeAdapter[JsonValue] = TypeAdapter(JsonValue)
# an unknown model makes LiteLLM print to stdout, corrupting the helper JSON
litellm.suppress_debug_info = True
MIXIN_MODEL = "accounts/fireworks/models/test-model"
RERANK_MODEL = "fireworks/qwen3-reranker-8b"
RERANK_URL = "https://api.fireworks.ai/inference/v1/rerank"
FIXED_START = datetime(2024, 1, 1, tzinfo=UTC)


class EmbeddingConfig(Protocol):
    def get_supported_openai_params(self, model: str) -> list[str]: ...

    def map_openai_params(
        self,
        non_default_params: dict[str, JsonValue],
        optional_params: dict[str, JsonValue],
        model: str,
    ) -> dict[str, JsonValue]: ...


class RerankConfig(Protocol):
    def get_complete_url(self, api_base: str | None, model: str) -> str: ...

    def validate_environment(
        self,
        headers: dict[str, str],
        model: str,
        api_key: str | None = None,
    ) -> dict[str, str]: ...

    def transform_rerank_request(
        self,
        model: str,
        optional_rerank_params: dict[str, JsonValue],
        headers: dict[str, str],
    ) -> dict[str, JsonValue]: ...

    def transform_rerank_response(
        self,
        model: str,
        raw_response: httpx.Response,
        model_response: RerankResponse,
        logging_obj: Logging,
    ) -> RerankResponse: ...


MixinValidate: TypeAlias = Callable[
    [
        dict[str, str],
        str,
        list[dict[str, JsonValue]],
        dict[str, JsonValue],
        dict[str, JsonValue],
        str | None,
    ],
    dict[str, str],
]
MIXIN_VALIDATE = cast(  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/fireworks_ai/common_utils.py#L92-L101
    MixinValidate, FireworksAIMixin().validate_environment
)
EMBEDDING: EmbeddingConfig = FireworksAIEmbeddingConfig()
RERANK: RerankConfig = FireworksAIRerankConfig()


EnvKeys: TypeAlias = dict[str, str | None]


class ApiKeyCall(TypedDict):
    argument: str | None
    env: EnvKeys


class SessionCall(TypedDict):
    litellm_params: dict[str, JsonValue]


class AffinityCall(TypedDict):
    headers: dict[str, str]
    litellm_params: dict[str, JsonValue]


class MixinCall(TypedDict):
    headers: dict[str, str]
    api_key: str | None
    env: EnvKeys
    litellm_params: dict[str, JsonValue]


class EmbeddingCall(TypedDict):
    model: str
    non_default_params: dict[str, JsonValue]
    optional_params: dict[str, JsonValue]


class PricingCall(TypedDict):
    model: str


class UsageFields(TypedDict):
    prompt_tokens: int
    completion_tokens: int
    total_tokens: int


class CostCall(TypedDict):
    model: str
    usage: UsageFields
    instant: str


class UrlCall(TypedDict):
    api_base: str | None


class HeadersCall(TypedDict):
    headers: dict[str, str]
    api_key: str | None
    env: EnvKeys


class RequestCall(TypedDict):
    model: str
    params: dict[str, JsonValue]


class ResponseCall(TypedDict):
    status: int
    headers: dict[str, str]
    body: str


class Input(TypedDict):
    model_cost: dict[str, dict[str, JsonValue]]
    api_keys: list[ApiKeyCall]
    sessions: list[SessionCall]
    affinity: list[AffinityCall]
    mixin_headers: list[MixinCall]
    embedding_params: list[EmbeddingCall]
    pricing: list[PricingCall]
    costs: list[CostCall]
    rerank_urls: list[UrlCall]
    rerank_headers: list[HeadersCall]
    rerank_requests: list[RequestCall]
    rerank_responses: list[ResponseCall]


def embedding_answer(call: EmbeddingCall) -> JsonValue:
    supported = EMBEDDING.get_supported_openai_params(call["model"])
    mapped = EMBEDDING.map_openai_params(
        call["non_default_params"], call["optional_params"], call["model"]
    )
    return {"supported": JSON.validate_python(supported), "mapped": mapped}


def cost_answer(call: CostCall) -> JsonValue:
    usage = Usage(
        prompt_tokens=call["usage"]["prompt_tokens"],
        completion_tokens=call["usage"]["completion_tokens"],
        total_tokens=call["usage"]["total_tokens"],
    )
    prompt_cost, completion_cost = cost_calculator.cost_per_token(
        call["model"], usage, datetime.fromisoformat(call["instant"])
    )
    return {
        "prompt_cost": format(Decimal(repr(prompt_cost)).normalize(), "f"),
        "completion_cost": format(Decimal(repr(completion_cost)).normalize(), "f"),
    }


def response_answer(call: ResponseCall) -> JsonValue:
    raw = httpx.Response(
        status_code=call["status"],
        headers=call["headers"],
        content=call["body"].encode(),
        request=httpx.Request("POST", RERANK_URL),
    )
    logging_obj = Logging(
        model=RERANK_MODEL,
        messages=None,
        stream=False,
        call_type="rerank",
        start_time=FIXED_START,
        litellm_call_id="00000000000040008000000000000001",
        function_id="00000000000040008000000000000001",
    )
    projected = RERANK.transform_rerank_response(
        model=RERANK_MODEL,
        raw_response=raw,
        model_response=RerankResponse(),
        logging_obj=logging_obj,
    )
    return JSON.validate_json(projected.model_dump_json())


def run(input_value: Input) -> JsonValue:
    litellm.model_cost = input_value["model_cost"]
    api_keys: list[str | None] = [
        common_utils.resolve_fireworks_api_key(call["argument"])
        for call in input_value["api_keys"]
    ]
    sessions: list[str | None] = [
        common_utils.get_fireworks_session_id(call["litellm_params"])
        for call in input_value["sessions"]
    ]
    affinity: list[dict[str, str]] = [
        dict(
            common_utils.with_fireworks_session_affinity(
                call["headers"], call["litellm_params"]
            )
        )
        for call in input_value["affinity"]
    ]
    mixin_headers: list[dict[str, str]] = [
        dict(
            MIXIN_VALIDATE(
                call["headers"],
                MIXIN_MODEL,
                [],
                {},
                call["litellm_params"],
                call["api_key"],
            )
        )
        for call in input_value["mixin_headers"]
    ]
    pricing: list[str] = [
        get_base_model_for_pricing(call["model"]) for call in input_value["pricing"]
    ]
    rerank_urls: list[str] = [
        RERANK.get_complete_url(call["api_base"], RERANK_MODEL)
        for call in input_value["rerank_urls"]
    ]
    rerank_headers: list[dict[str, str]] = [
        dict(
            RERANK.validate_environment(
                headers=call["headers"],
                model=RERANK_MODEL,
                api_key=call["api_key"],
            )
        )
        for call in input_value["rerank_headers"]
    ]
    rerank_requests: list[dict[str, JsonValue]] = [
        RERANK.transform_rerank_request(call["model"], call["params"], {})
        for call in input_value["rerank_requests"]
    ]
    answer: dict[str, JsonValue] = {}
    answer["api_keys"] = JSON.validate_python(api_keys)
    answer["sessions"] = JSON.validate_python(sessions)
    answer["affinity"] = JSON.validate_python(affinity)
    answer["mixin_headers"] = JSON.validate_python(mixin_headers)
    answer["embedding_params"] = [
        embedding_answer(call) for call in input_value["embedding_params"]
    ]
    answer["pricing"] = JSON.validate_python(pricing)
    answer["costs"] = [cost_answer(call) for call in input_value["costs"]]
    answer["rerank_urls"] = JSON.validate_python(rerank_urls)
    answer["rerank_headers"] = JSON.validate_python(rerank_headers)
    answer["rerank_requests"] = JSON.validate_python(rerank_requests)
    answer["rerank_responses"] = [
        response_answer(call) for call in input_value["rerank_responses"]
    ]
    return answer
