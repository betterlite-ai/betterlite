import litellm
import httpx
from litellm.llms.custom_httpx.http_handler import HTTPHandler
from litellm.llms.huggingface.rerank.transformation import HuggingFaceRerankConfig
from litellm.llms.replicate.chat import handler as replicate_handler
from pydantic import JsonValue, TypeAdapter
from typing import NotRequired, TypedDict

litellm.suppress_debug_info = True


class Reply(TypedDict):
    status: int
    body: str


class Case(TypedDict):
    op: str
    model: str
    messages: list[dict[str, JsonValue]]
    optional_params: dict[str, JsonValue]
    stream: bool
    input_type: NotRequired[str]
    api_base: NotRequired[str | None]
    api_key: NotRequired[str | None]
    tenant_id: NotRequired[str]
    input: list[str]
    env: dict[str, str]
    replies: list[Reply]
    polls: list[Reply]


class Responder:
    def __init__(self, replies: list[Reply]) -> None:
        self.replies = list(replies)
        self.requests: list[JsonValue] = []

    def handle(self, request: httpx.Request) -> httpx.Response:
        body: JsonValue = None
        if request.content:
            body = TypeAdapter(JsonValue).validate_json(request.content)
        self.requests.append(
            {
                "method": request.method,
                "url": str(request.url),
                "headers": header_value(request.headers),
                "body": body,
            }
        )
        reply = self.replies.pop(0)
        return httpx.Response(reply["status"], content=reply["body"].encode())


class ClientFactory:
    def __init__(self, client: HTTPHandler) -> None:
        self.client = client

    def get(self, params: dict[str, float] | None = None) -> HTTPHandler:
        return self.client


def header_value(headers: httpx.Headers) -> dict[str, str]:
    return {
        name: value
        for name, value in headers.items()
        if name in ("authorization", "content-type", "accept") or name.startswith("x-")
    }


def responder_for(case: Case) -> Responder:
    return Responder([*case["replies"], *case["polls"]])


def client_for(responder: Responder) -> HTTPHandler:
    return HTTPHandler(client=httpx.Client(transport=httpx.MockTransport(responder.handle)))


def chat_value(case: Case) -> JsonValue:
    responder = responder_for(case)
    client = client_for(responder)
    litellm.module_level_client = client
    litellm.completion(
        model=f"huggingface/{case['model']}",
        messages=case["messages"],
        api_base=case.get("api_base"),
        api_key=case.get("api_key"),
        stream=case["stream"],
        client=client,
    )
    kind = "stream" if case["stream"] else "chat"
    return {"kind": kind, "requests": responder.requests}


def embedding_value(case: Case) -> JsonValue:
    responder = responder_for(case)
    client = client_for(responder)
    litellm.module_level_client = client
    litellm.embedding(
        model=f"huggingface/{case['model']}",
        input=case["input"],
        input_type=case.get("input_type"),
        api_base=case.get("api_base"),
        api_key=case.get("api_key"),
        client=client,
    )
    return {"requests": responder.requests}


def rerank_value(case: Case) -> JsonValue:
    config = HuggingFaceRerankConfig()
    api_base = case.get("api_base")
    headers = config.validate_environment(
        headers={},
        model=case["model"],
        api_key=case.get("api_key"),
        optional_params=None,
        litellm_params=None,
        api_base=api_base,
    )
    url = config.get_complete_url(api_base, case["model"], None)
    return {"url": url, "headers": header_value(httpx.Headers(headers))}


def replicate_value(case: Case) -> JsonValue:
    responder = responder_for(case)
    client = client_for(responder)
    factory = ClientFactory(client)
    replicate_handler._get_httpx_client = factory.get  # type: ignore[assignment]  # pyright: ignore[reportPrivateUsage]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/replicate/chat/handler.py#L186
    litellm.completion(
        model=f"replicate/{case['model']}",
        messages=case["messages"],
        api_base=case.get("api_base"),
        api_key=case.get("api_key"),
    )
    return {"requests": responder.requests}


def predibase_value(case: Case) -> JsonValue:
    responder = responder_for(case)
    client = client_for(responder)
    litellm.module_level_client = client
    tenant = case.get("tenant_id")
    if tenant is None:
        litellm.completion(
            model=f"predibase/{case['model']}",
            messages=case["messages"],
            api_base=case.get("api_base"),
            api_key=case.get("api_key"),
            client=client,
        )
    else:
        litellm.completion(
            model=f"predibase/{case['model']}",
            messages=case["messages"],
            api_base=case.get("api_base"),
            api_key=case.get("api_key"),
            client=client,
            tenant_id=tenant,
        )
    return {"requests": responder.requests}


def run(input_value: dict[str, JsonValue]) -> JsonValue:
    case = TypeAdapter(Case).validate_python(input_value)
    op = case["op"]
    if op == "huggingface_chat":
        return chat_value(case)
    if op == "huggingface_embedding":
        return embedding_value(case)
    if op == "huggingface_rerank":
        return rerank_value(case)
    if op == "replicate":
        return replicate_value(case)
    if op == "predibase":
        return predibase_value(case)
    raise ValueError("unknown operation")
