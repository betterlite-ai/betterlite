from typing import Final, Literal, NotRequired, Required, TypedDict

import litellm
from pydantic import JsonValue, TypeAdapter

import bedrock_image_dispatch
import bedrock_other_count_call
import bedrock_other_embedding_call
import bedrock_other_embedding_jobs
import bedrock_other_rerank
import bedrock_other_transcription

NOW: Final = 1_700_000_000
UNSUPPORTED_FORMAT: Final = "unsupported_format"

JSON: Final[TypeAdapter[JsonValue]] = TypeAdapter(JsonValue)

Case = Literal[
    "embedding",
    "embedding_status",
    "rerank",
    "count",
    "transcription",
    "image_generation",
    "image_edit",
]


class Reply(TypedDict, total=False):
    status: int
    headers: dict[str, str]
    body: str
    failure: bool


class File(TypedDict):
    content: str
    filename: NotRequired[str]


class Probe(TypedDict, total=False):
    case: Required[Case]
    model: Required[str]
    input: list[str]
    prompt: str | None
    images: list[str]
    optional_params: dict[str, JsonValue]
    query: str | None
    documents: list[JsonValue]
    top_n: int | None
    return_documents: bool | None
    rank_fields: list[str] | None
    messages: list[dict[str, JsonValue]] | None
    request_model: str | None
    invocation_arn: str | None
    region: str | None
    environment: dict[str, str]
    file: File | None
    timeout: float | None
    timeout_ms: int | None
    reply_text: str | None
    asynchronous: bool
    api_base: str | None
    api_key: str | None
    extra_headers: dict[str, str] | None
    reply: Reply
    replies: list[Reply]


class Input(TypedDict):
    model_cost: NotRequired[dict[str, JsonValue]]
    probes: list[Probe]


class EmbeddingRequest(TypedDict):
    url: str
    headers: dict[str, str]
    body: JsonValue


class EmbeddingRecord(TypedDict):
    index: int
    embedding: JsonValue


class EmbeddingUsage(TypedDict):
    prompt_tokens: int
    total_tokens: int


class EmbeddingResponse(TypedDict):
    model: str | None
    data: list[EmbeddingRecord]
    usage: EmbeddingUsage


class EmbeddingOutput(TypedDict):
    ok: bool
    status: int | None
    requests: list[EmbeddingRequest]
    response: EmbeddingResponse | None


class StatusRequest(TypedDict):
    url: str
    headers: list[list[str]]
    body: str


class StatusOutput(TypedDict):
    requests: list[StatusRequest]
    resolved_regions: list[str]
    kind: Literal["returned", "failed"]
    value: JsonValue


class SignedHeaders(TypedDict):
    authorization: bool


class RerankRequest(TypedDict):
    posted_url: str
    posted_body: JsonValue
    signed_headers: SignedHeaders


class RerankResult(TypedDict):
    index: int
    relevance_score: float


class RerankResponse(TypedDict):
    id: str | None
    results: list[RerankResult] | None


class CountRequest(TypedDict):
    url: str
    body: str
    headers: dict[str, str]


class CountResult(TypedDict):
    total_tokens: int
    request_model: str
    model_used: str
    tokenizer_type: str
    error: bool
    error_message: str | None
    status_code: int | None
    original_response: JsonValue


class CountCall(TypedDict):
    result: CountResult | None
    request: CountRequest | None


class CountOutput(TypedDict):
    calls: list[CountCall]


class TranscriptionResult(TypedDict):
    ok: bool
    seen: list[JsonValue]
    text: NotRequired[str]
    kind: NotRequired[str]


class TranscriptionOutput(TypedDict):
    calls: list[TranscriptionResult]


class ImageRequest(TypedDict):
    url: str | None
    body: JsonValue
    authorization: str | None
    posted: int


class ImageRow(TypedDict):
    request: ImageRequest
    ok: bool
    status: int | None
    images: list[JsonValue]
    cost: JsonValue


INPUT: Final[TypeAdapter[Input]] = TypeAdapter(Input)
JOBS_INPUT: Final[TypeAdapter[bedrock_other_embedding_jobs.Input]] = TypeAdapter(
    bedrock_other_embedding_jobs.Input
)
RERANK_PROBE: Final[TypeAdapter[bedrock_other_rerank.Probe]] = TypeAdapter(
    bedrock_other_rerank.Probe
)
COUNT_INPUT: Final[TypeAdapter[dict[str, bedrock_other_count_call.Calls]]] = TypeAdapter(
    dict[str, bedrock_other_count_call.Calls]
)
TRANSCRIPTION_INPUT: Final[TypeAdapter[bedrock_other_transcription.Calls]] = TypeAdapter(
    bedrock_other_transcription.Calls
)
IMAGE_INPUT: Final[TypeAdapter[bedrock_image_dispatch.Input]] = TypeAdapter(
    bedrock_image_dispatch.Input
)
EMBEDDING_OUTPUT: Final[TypeAdapter[EmbeddingOutput]] = TypeAdapter(EmbeddingOutput)
STATUS_OUTPUT: Final[TypeAdapter[StatusOutput]] = TypeAdapter(StatusOutput)
RERANK_REQUEST: Final[TypeAdapter[RerankRequest]] = TypeAdapter(RerankRequest)
RERANK_OUTPUT: Final[TypeAdapter[RerankResponse]] = TypeAdapter(RerankResponse)
COUNT_OUTPUT: Final[TypeAdapter[CountOutput]] = TypeAdapter(CountOutput)
TRANSCRIPTION_OUTPUT: Final[TypeAdapter[TranscriptionOutput]] = TypeAdapter(
    TranscriptionOutput
)
IMAGE_OUTPUT: Final[TypeAdapter[list[ImageRow]]] = TypeAdapter(list[ImageRow])


def params_of(probe: Probe) -> dict[str, JsonValue]:
    params = probe.get("optional_params")
    if params is None:
        return {}
    return params


def environment_of(probe: Probe) -> dict[str, str]:
    environment = probe.get("environment")
    if environment is None:
        return {}
    return environment


def input_values(probe: Probe) -> list[str]:
    values = probe.get("input")
    if values is None:
        return []
    return values


def query_value(probe: Probe) -> str:
    query = probe.get("query")
    if query is None:
        return ""
    return query


def request_model_of(probe: Probe) -> str:
    request_model = probe.get("request_model")
    if request_model is None:
        return probe["model"]
    return request_model


def probe_images(probe: Probe) -> list[str]:
    images = probe.get("images")
    if images is None:
        return []
    return images


def replies_of(probe: Probe) -> list[Reply]:
    replies = probe.get("replies")
    if replies is not None:
        return replies
    reply = probe.get("reply")
    if reply is None:
        empty: Reply = {}
        return [empty]
    return [reply]


def reply_status(reply: Reply | None) -> int:
    if reply is None:
        return 200
    status = reply.get("status")
    if status is None:
        return 200
    return status


def reply_body(reply: Reply | None) -> str:
    if reply is None:
        return "{}"
    body = reply.get("body")
    if body is None:
        return "{}"
    return body


def reply_headers(reply: Reply | None) -> JsonValue:
    if reply is None:
        return JSON.validate_python({})
    headers = reply.get("headers")
    if headers is None:
        return JSON.validate_python({})
    return JSON.validate_python(headers)


def keys_of(probe: Probe) -> dict[str, JsonValue]:
    params = params_of(probe)
    return {
        "access_key_id": params.get("aws_access_key_id"),
        "secret_access_key": params.get("aws_secret_access_key"),
    }


def credentials_of(probe: Probe) -> dict[str, JsonValue]:
    keys = keys_of(probe)
    keys["session_token"] = None
    return keys


def header_marker(value: str | None) -> str | None:
    if value is None:
        return None
    if value.startswith("AWS4-HMAC-SHA256"):
        return "sigv4"
    return "other"


def status_header(headers: list[list[str]]) -> str | None:
    for pair in headers:
        if pair[0].lower() == "authorization":
            return header_marker(pair[1])
    return None


def parsed_body(text: str) -> JsonValue:
    if text == "":
        return None
    return JSON.validate_json(text)


def request_row(url: str, body: JsonValue, authorization: str | None) -> JsonValue:
    raw: dict[str, JsonValue] = {
        "url": url,
        "body": body,
        "authorization": authorization,
    }
    return JSON.validate_python(raw)


def success_row(requests: list[JsonValue], value: JsonValue) -> JsonValue:
    raw: dict[str, JsonValue] = {
        "requests": requests,
        "ok": True,
        "status": None,
        "error": None,
        "value": value,
    }
    return JSON.validate_python(raw)


def failure_row(requests: list[JsonValue], status: int | None, error: str | None) -> JsonValue:
    raw: dict[str, JsonValue] = {
        "requests": requests,
        "ok": False,
        "status": status,
        "error": error,
        "value": None,
    }
    return JSON.validate_python(raw)


def embedding_value(response: EmbeddingResponse) -> JsonValue:
    data: list[JsonValue] = [
        JSON.validate_python({"index": record["index"], "embedding": record["embedding"]})
        for record in response["data"]
    ]
    usage: dict[str, JsonValue] = {
        "prompt_tokens": response["usage"]["prompt_tokens"],
        "total_tokens": response["usage"]["total_tokens"],
    }
    raw: dict[str, JsonValue] = {
        "model": response["model"],
        "data": data,
        "usage": usage,
    }
    return JSON.validate_python(raw)


def embedding_reply(reply: Reply) -> dict[str, JsonValue]:
    raw: dict[str, JsonValue] = {
        "status": reply_status(reply),
        "body": reply_body(reply),
        "headers": reply_headers(reply),
    }
    if reply.get("failure") is True:
        raw["failure"] = "failed"
    return raw


def embedding_case(probe: Probe) -> dict[str, JsonValue]:
    replies: list[JsonValue] = [
        JSON.validate_python(embedding_reply(reply)) for reply in replies_of(probe)
    ]
    raw: dict[str, JsonValue] = {
        "function": "embeddings",
        "model": probe["model"],
        "input": JSON.validate_python(input_values(probe)),
        "now": NOW,
        "optional_params": params_of(probe),
        "api_base": probe.get("api_base"),
        "extra_headers": JSON.validate_python(probe.get("extra_headers")),
        "api_key": probe.get("api_key"),
        "timeout": probe.get("timeout"),
        "aembedding": False,
        "drop_params": False,
        "supplied": "sync",
        "environment": JSON.validate_python(environment_of(probe)),
        "replies": JSON.validate_python(replies),
    }
    return raw


def embedding_request(request: EmbeddingRequest) -> JsonValue:
    authorization = header_marker(request["headers"].get("authorization"))
    return request_row(request["url"], request["body"], authorization)


def embedding_row(probe: Probe) -> JsonValue:
    output = EMBEDDING_OUTPUT.validate_python(
        bedrock_other_embedding_call.run(embedding_case(probe))
    )
    requests: list[JsonValue] = [
        embedding_request(request) for request in output["requests"]
    ]
    if not output["ok"]:
        return failure_row(requests, output["status"], None)
    value: JsonValue = None
    response = output["response"]
    if response is not None:
        value = embedding_value(response)
    return success_row(requests, value)


def region_of(probe: Probe) -> str:
    region = probe.get("region")
    if region is None:
        return "us-east-1"
    return region


def count_environment(probe: Probe) -> dict[str, JsonValue]:
    environment = environment_of(probe)
    return {
        "region_name": environment.get("AWS_REGION_NAME"),
        "region": environment.get("AWS_REGION"),
        "session_region": environment.get("AWS_DEFAULT_REGION"),
        "runtime_endpoint": environment.get("AWS_BEDROCK_RUNTIME_ENDPOINT"),
        "bearer_token": environment.get("AWS_BEDROCK_BEARER_TOKEN"),
    }


def status_request(probe: Probe) -> dict[str, JsonValue]:
    reply = probe.get("reply")
    raw: dict[str, JsonValue] = {
        "function": "status",
        "invocation_arn": probe.get("invocation_arn"),
        "region": region_of(probe),
        "environment": JSON.validate_python(environment_of(probe)),
        "credentials": JSON.validate_python(keys_of(probe)),
        "status": reply_status(reply),
        "body": reply_body(reply),
    }
    endpoint = params_of(probe).get("aws_bedrock_runtime_endpoint")
    if endpoint is not None:
        raw["endpoint"] = endpoint
    return raw


def status_value(output: StatusOutput) -> JsonValue:
    raw: dict[str, JsonValue] = {
        "body": output["value"],
        "regions": JSON.validate_python(output["resolved_regions"]),
    }
    return JSON.validate_python(raw)


def status_request_row(request: StatusRequest) -> JsonValue:
    body = parsed_body(request["body"])
    return request_row(request["url"], body, status_header(request["headers"]))


def status_row(probe: Probe) -> JsonValue:
    output = STATUS_OUTPUT.validate_python(
        bedrock_other_embedding_jobs.run(JOBS_INPUT.validate_python(status_request(probe)))
    )
    requests: list[JsonValue] = [
        status_request_row(request) for request in output["requests"]
    ]
    if output["kind"] == "returned":
        return success_row(requests, status_value(output))
    return failure_row(requests, None, None)


def rerank_reply(probe: Probe) -> dict[str, JsonValue]:
    reply = probe.get("reply")
    failure: JsonValue = None
    if reply is not None and reply.get("failure") is True:
        failure = "failed"
    return {
        "status": reply_status(reply),
        "headers": reply_headers(reply),
        "body": reply_body(reply),
        "failure": failure,
    }


def rerank_probe(probe: Probe) -> dict[str, JsonValue]:
    documents = probe.get("documents")
    if documents is None:
        documents = []
    raw: dict[str, JsonValue] = {
        "model": probe["model"],
        "query": query_value(probe),
        "documents": JSON.validate_python(documents),
        "top_n": probe.get("top_n"),
        "rank_fields": JSON.validate_python(probe.get("rank_fields")),
        "return_documents": probe.get("return_documents"),
        "max_chunks_per_doc": None,
        "optional_params": params_of(probe),
        "extra_headers": JSON.validate_python(probe.get("extra_headers")),
        "api_base": probe.get("api_base"),
        "credentials": JSON.validate_python(credentials_of(probe)),
        "reply": JSON.validate_python(rerank_reply(probe)),
        "supplied": "sync",
        "is_async": False,
        "timeout_ms": probe.get("timeout_ms"),
    }
    return raw


def rerank_value(response: RerankResponse) -> JsonValue:
    results: JsonValue = None
    items = response["results"]
    if items is not None:
        entries: list[JsonValue] = [
            JSON.validate_python({"index": item["index"], "score": item["relevance_score"]})
            for item in items
        ]
        results = JSON.validate_python(entries)
    raw: dict[str, JsonValue] = {"id": response["id"], "results": results}
    return JSON.validate_python(raw)


def rerank_row(probe: Probe) -> JsonValue:
    request = RERANK_REQUEST.validate_python(
        bedrock_other_rerank.prepare_value(RERANK_PROBE.validate_python(rerank_probe(probe)))
    )
    response = RERANK_OUTPUT.validate_python(
        bedrock_other_rerank.result_value(RERANK_PROBE.validate_python(rerank_probe(probe)))
    )
    authorization: str | None = None
    if request["signed_headers"]["authorization"]:
        authorization = "sigv4"
    requests: list[JsonValue] = [
        request_row(request["posted_url"], request["posted_body"], authorization)
    ]
    return success_row(requests, rerank_value(response))


def count_reply(probe: Probe) -> dict[str, JsonValue]:
    reply = probe.get("reply")
    failure = reply is not None and reply.get("failure") is True
    return {
        "status": reply_status(reply),
        "body": reply_body(reply),
        "headers": reply_headers(reply),
        "failure": failure,
    }


def count_call(probe: Probe) -> JsonValue:
    deployment: dict[str, JsonValue] = {"litellm_params": params_of(probe)}
    raw: dict[str, JsonValue] = {
        "model": probe["model"],
        "now": NOW,
        "messages": JSON.validate_python(probe.get("messages")),
        "request_model": request_model_of(probe),
        "deployment": JSON.validate_python(deployment),
        "environment": JSON.validate_python(count_environment(probe)),
        "reply": JSON.validate_python(count_reply(probe)),
    }
    return JSON.validate_python(raw)


def count_value(result: CountResult | None) -> JsonValue:
    if result is None:
        return None
    raw: dict[str, JsonValue] = {
        "total_tokens": result["total_tokens"],
        "request_model": result["request_model"],
        "model_used": result["model_used"],
        "tokenizer_type": result["tokenizer_type"],
        "error": result["error"],
        "error_message": result["error_message"],
        "status_code": result["status_code"],
        "original_response": result["original_response"],
    }
    return JSON.validate_python(raw)


def count_request(request: CountRequest) -> JsonValue:
    authorization = header_marker(request["headers"].get("authorization"))
    return request_row(request["url"], parsed_body(request["body"]), authorization)


def count_row(probe: Probe) -> JsonValue:
    raw: dict[str, JsonValue] = {
        "count_token_http_request": JSON.validate_python({"calls": [count_call(probe)]})
    }
    output = COUNT_OUTPUT.validate_python(
        bedrock_other_count_call.run(COUNT_INPUT.validate_python(raw))
    )
    call = output["calls"][0]
    requests: list[JsonValue] = []
    request = call["request"]
    if request is not None:
        requests.append(count_request(request))
    return success_row(requests, count_value(call["result"]))


def file_case(file_value: File) -> JsonValue:
    content = file_value["content"]
    filename = file_value.get("filename")
    if filename is None:
        return JSON.validate_python({"kind": "bytes", "content": content})
    return JSON.validate_python({"kind": "tuple", "filename": filename, "content": content})


def transcription_call(probe: Probe) -> dict[str, JsonValue]:
    file_value = probe.get("file")
    if file_value is None:
        raise ValueError("a transcription probe carries a file")
    mode = "async" if probe.get("asynchronous") else "sync"
    raw: dict[str, JsonValue] = {
        "mode": mode,
        "model": probe["model"],
        "file": file_case(file_value),
        "custom_llm_provider": "bedrock",
        "optional_params": params_of(probe),
        "api_key": probe.get("api_key"),
        "api_base": probe.get("api_base"),
        "extra_headers": JSON.validate_python(probe.get("extra_headers")),
        "timeout": probe.get("timeout"),
        "reply_text": probe.get("reply_text"),
    }
    return raw


def transcription_row(probe: Probe) -> JsonValue:
    raw: dict[str, JsonValue] = {"calls": JSON.validate_python([transcription_call(probe)])}
    output = TRANSCRIPTION_OUTPUT.validate_python(
        bedrock_other_transcription.run(TRANSCRIPTION_INPUT.validate_python(raw))
    )
    call = output["calls"][0]
    if not call["ok"]:
        return failure_row([], None, UNSUPPORTED_FORMAT)
    text = call.get("text")
    if text is None:
        text = ""
    return success_row([], JSON.validate_python({"text": text, "seen": call["seen"]}))


def image_reply(probe: Probe) -> dict[str, JsonValue]:
    reply = probe.get("reply")
    return {
        "status": reply_status(reply),
        "headers": reply_headers(reply),
        "body": reply_body(reply),
    }


def image_probe(probe: Probe) -> dict[str, JsonValue]:
    raw: dict[str, JsonValue] = {
        "model": probe["model"],
        "prompt": probe.get("prompt"),
        "images": JSON.validate_python(probe_images(probe)),
        "optional_params": params_of(probe),
        "extra_headers": JSON.validate_python(probe.get("extra_headers")),
        "api_base": probe.get("api_base"),
        "api_key": probe.get("api_key"),
        "reply": JSON.validate_python(image_reply(probe)),
    }
    return raw


def image_value(row: ImageRow) -> JsonValue:
    raw: dict[str, JsonValue] = {"images": row["images"], "cost": row["cost"]}
    return JSON.validate_python(raw)


def image_row(
    model_cost: dict[str, JsonValue] | None,
    probe: Probe,
    case: Literal["canvas_image", "canvas_edit"],
) -> JsonValue:
    cost_map: dict[str, JsonValue] = {}
    if model_cost is not None:
        cost_map = model_cost
    litellm.model_cost = cost_map
    raw: dict[str, JsonValue] = {
        "case": case,
        "model_cost": cost_map,
        "probes": JSON.validate_python([image_probe(probe)]),
    }
    output = IMAGE_OUTPUT.validate_python(
        bedrock_image_dispatch.run(IMAGE_INPUT.validate_python(raw))
    )
    row = output[0]
    requests: list[JsonValue] = []
    request = row["request"]
    url = request["url"]
    if url is not None:
        requests.append(request_row(url, request["body"], request["authorization"]))
    if row["ok"]:
        return success_row(requests, image_value(row))
    return failure_row(requests, row["status"], None)


def probe_row(model_cost: dict[str, JsonValue] | None, probe: Probe) -> JsonValue:
    row: JsonValue = None
    match probe["case"]:
        case "embedding":
            row = embedding_row(probe)
        case "embedding_status":
            row = status_row(probe)
        case "rerank":
            row = rerank_row(probe)
        case "count":
            row = count_row(probe)
        case "transcription":
            row = transcription_row(probe)
        case "image_generation":
            row = image_row(model_cost, probe, "canvas_image")
        case "image_edit":
            row = image_row(model_cost, probe, "canvas_edit")
    return row


def run(input_value: dict[str, JsonValue]) -> JsonValue:
    values = INPUT.validate_python(input_value)
    model_cost = values.get("model_cost")
    rows: list[JsonValue] = [
        probe_row(model_cost, probe) for probe in values["probes"]
    ]
    return JSON.validate_python(rows)
