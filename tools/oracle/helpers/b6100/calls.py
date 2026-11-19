import httpx
import litellm
from litellm.exceptions import AuthenticationError
from litellm.llms.custom_httpx.http_handler import HTTPHandler
from litellm.llms.edenai.common_utils import (
    EdenAIException,
    require_api_key,
    resolve_api_base,
    resolve_api_key,
)
from litellm.types.router import GenericLiteLLMParams
from pydantic import JsonValue
from typing_extensions import override

from .configs import CHAT, EMBEDDING, MESSAGES, RESPONSES
from .wire import MODEL, Call, CatalogCase, ChatParams, EmbeddingPlan, apply_environment


class CatalogTransport(httpx.BaseTransport):
    def __init__(self, *, status: int, text: str, headers: dict[str, str]) -> None:
        self.status = status
        self.text = text
        self.headers = headers

    @override
    def handle_request(self, request: httpx.Request) -> httpx.Response:
        return httpx.Response(
            status_code=self.status,
            headers=self.headers,
            content=self.text.encode(),
            request=request,
        )


def credentials_value(case: Call) -> JsonValue:
    apply_environment(case["environment"], case["global_api_key"])
    explicit = case["api_key"]
    required: JsonValue
    try:
        found = require_api_key(explicit, MODEL)
    except AuthenticationError as error:  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/exceptions.py#L132
        required = {"key": None, "status": error.status_code}
    else:
        required = {"key": found, "status": None}
    return {
        "api_base": resolve_api_base(case["api_base"]),
        "api_key": resolve_api_key(explicit),
        "required": required,
    }


def chat_params_value(case: ChatParams) -> JsonValue:
    litellm.model_cost = {case["model"]: {"supports_reasoning": case["reasoning"]}}  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/__init__.py#L563
    return list[JsonValue](CHAT.get_supported_openai_params(case["model"]))


def catalog_value(case: CatalogCase) -> JsonValue:
    apply_environment(case["environment"], None)
    url = f"{resolve_api_base(case['api_base'])}/models"
    transport = CatalogTransport(
        status=case["status"], text=case["text"], headers=case["headers"]
    )
    litellm.module_level_client = HTTPHandler(client=httpx.Client(transport=transport))
    try:
        models = CHAT.get_models(api_base=case["api_base"])
    except EdenAIException as error:  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/edenai/common_utils.py#L22
        return {"url": url, "status": error.status_code}
    except ValueError:
        return {"url": url, "status": 500}
    return {"url": url, "models": list[JsonValue](models)}


def embedding_plan_value(case: EmbeddingPlan) -> JsonValue:
    apply_environment(case["environment"], case["global_api_key"])
    headers = EMBEDDING.validate_environment(
        case["headers"], MODEL, [], case["optional_params"], {}, case["api_key"]
    )
    return {
        "params": EMBEDDING.map_openai_params(
            case["non_default_params"], case["optional_params"], MODEL, drop_params=False
        ),
        "url": EMBEDDING.get_complete_url(
            case["api_base"], None, MODEL, case["optional_params"], {}
        ),
        "headers": headers,
        "body": EMBEDDING.transform_embedding_request(
            MODEL, case["input"], case["optional_params"], headers
        ),
    }


def prepare_value(case: Call) -> JsonValue:
    apply_environment(case["environment"], case["global_api_key"])
    params = GenericLiteLLMParams(api_key=case["api_key"])
    try:
        headers = RESPONSES.validate_environment(case["headers"], MODEL, params)
    except AuthenticationError as error:  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/exceptions.py#L132
        return {"status": error.status_code}
    return {
        "url": RESPONSES.get_complete_url(case["api_base"], {}),
        "headers": headers,
        "should_fake_stream": RESPONSES.should_fake_stream(MODEL, stream=False),
        "supports_native_websocket": RESPONSES.supports_native_websocket(),
    }


def messages_environment_value(case: Call) -> JsonValue:
    apply_environment(case["environment"], case["global_api_key"])
    try:
        headers, _api_base = MESSAGES.validate_anthropic_messages_environment(
            case["headers"], MODEL, [], {}, {}, case["api_key"], case["api_base"]
        )
    except AuthenticationError as error:  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/exceptions.py#L132
        return {"status": error.status_code}
    url = MESSAGES.get_complete_url(case["api_base"], None, MODEL, {}, {})
    return {"headers": headers, "url": url}
