import asyncio
import base64
import math
from dataclasses import dataclass, field
from functools import partial
from typing import Literal, NotRequired, TypedDict, cast

import httpx
from litellm.llms.custom_httpx.http_handler import httpxSpecialProvider
from litellm.rag.ingestion import vertex_ai_ingestion
from litellm.router import Router
from litellm.types.llms.vertex_ai import VERTEX_CREDENTIALS_TYPES
from litellm.types.rag import RAGIngestOptions
from pydantic import JsonValue, TypeAdapter
from typing_extensions import override


class OptionsInput(TypedDict):
    name: NotRequired[str]
    chunking_strategy: NotRequired[dict[str, JsonValue]]
    vector_store: NotRequired[dict[str, JsonValue]]


class FileInput(TypedDict):
    content_base64: NotRequired[str]
    filename: NotRequired[str]
    content_type: NotRequired[str]


class ReplyInput(TypedDict):
    status: int
    body: str


class Case(TypedDict):
    options: OptionsInput
    file: NotRequired[FileInput]
    responses: NotRequired[list[ReplyInput]]


class Multipart(TypedDict):
    metadata: JsonValue
    filename: str
    file_bytes: int
    content_type: str


CASE: TypeAdapter[Case] = TypeAdapter(Case)
JSON: TypeAdapter[JsonValue] = TypeAdapter(JsonValue)
MULTIPART_FILES = dict[str, tuple[str | None, str | bytes, str]]


@dataclass
class Script:
    replies: list[ReplyInput]
    requests: list[JsonValue] = field(default_factory=list)
    slept: list[float] = field(default_factory=list)

    def answer(self) -> httpx.Response:
        if not self.replies:
            raise RuntimeError("the reply script is exhausted")
        reply = self.replies.pop(0)
        return httpx.Response(reply["status"], content=reply["body"].encode())

    def __call__(self, delay: float) -> None:
        self.slept.append(delay)


class FakeClient:
    def __init__(self, script: Script, provider: httpxSpecialProvider, timeout: float) -> None:
        self.script = script
        self.provider = provider
        self.timeout = timeout

    async def post(
        self,
        url: str,
        json: JsonValue = None,
        files: MULTIPART_FILES | None = None,
        headers: dict[str, str] | None = None,  # noqa: ARG002  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/rag/ingestion/vertex_ai_ingestion.py#L212-L219
    ) -> httpx.Response:
        kind = "post_json" if files is None else "post_multipart"
        self.script.requests.append(request_record(kind, url, self.timeout, json, files))
        return self.script.answer()

    async def get(
        self,
        url: str,
        headers: dict[str, str] | None = None,  # noqa: ARG002  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/rag/ingestion/vertex_ai_ingestion.py#L281-L286
    ) -> httpx.Response:
        self.script.requests.append(request_record("get", url, self.timeout, None, None))
        return self.script.answer()


def request_record(
    kind: str,
    url: str,
    timeout: float,
    body: JsonValue,
    files: MULTIPART_FILES | None,
) -> dict[str, JsonValue]:
    parts = None if files is None else multipart_parts(files)
    return {
        "kind": kind,
        "url": url,
        "timeout": timeout,
        "json": body,
        "metadata": None if parts is None else parts["metadata"],
        "filename": None if parts is None else parts["filename"],
        "file_bytes": None if parts is None else parts["file_bytes"],
        "file_content_type": None if parts is None else parts["content_type"],
    }


def multipart_parts(files: MULTIPART_FILES) -> Multipart:
    metadata = files["metadata"]
    part = files["file"]
    filename = part[0]
    if filename is None:
        raise ValueError("the multipart part carries no filename")
    return {
        "metadata": JSON.validate_json(metadata[1]),
        "filename": filename,
        "file_bytes": len(part[1]),
        "content_type": part[2],
    }


def make_client(
    script: Script,
    llm_provider: httpxSpecialProvider,
    params: dict[str, float] | None = None,
) -> FakeClient:
    timeout = 60.0 if params is None else params["timeout"]
    return FakeClient(script, llm_provider, timeout)


def install(script: Script) -> None:
    vertex_ai_ingestion.get_async_httpx_client = partial(make_client, script)  # type: ignore[assignment, attr-defined]  # pyright: ignore[reportAssignmentType, reportAttributeAccessIssue]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/rag/ingestion/vertex_ai_ingestion.py#L207-L210
    asyncio.sleep = script  # type: ignore[assignment]  # pyright: ignore[reportAssignmentType, reportAttributeAccessIssue]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/rag/ingestion/vertex_ai_ingestion.py#L310


class Ingestion(vertex_ai_ingestion.VertexAIRAGIngestion):
    @override
    def __init__(self, ingest_options: RAGIngestOptions, router: Router | None = None) -> None:
        super().__init__(ingest_options, router)
        self.token_calls: list[JsonValue] = []

    @override
    def _ensure_access_token(
        self,
        credentials: VERTEX_CREDENTIALS_TYPES | None,
        project_id: str | None,
        custom_llm_provider: Literal["vertex_ai", "vertex_ai_beta", "gemini"],  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/vertex_ai/vertex_llm_base.py#L602-L609
    ) -> tuple[str, str]:
        self.token_calls.append({"project": project_id, "credentials": credentials is not None})
        if project_id is None:
            raise ValueError("the ingestion resolved no project")
        return "oracle-token", project_id


def ingest_options(options: OptionsInput) -> RAGIngestOptions:
    fields: dict[str, JsonValue] = {}
    fields["name"] = options.get("name")
    fields["chunking_strategy"] = options.get("chunking_strategy")
    fields["vector_store"] = options.get("vector_store")
    return cast(  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/rag.py#L183
        RAGIngestOptions, fields
    )


def file_input(case: Case) -> FileInput:
    supplied = case.get("file")
    if supplied is None:
        return {}
    return supplied


def run(input_value: dict[str, JsonValue]) -> JsonValue:
    case = CASE.validate_python(input_value)
    responses = case.get("responses")
    script = Script([] if responses is None else list(responses))
    install(script)
    file = file_input(case)
    encoded = file.get("content_base64")
    content = None if encoded is None else base64.b64decode(encoded)
    ingestion = Ingestion(ingest_options(case["options"]))
    corpus_id: str | None = None
    file_id: str | None = None
    failure: JsonValue = None
    try:
        corpus_id, file_id = asyncio.run(
            ingestion.store(
                file_content=content,
                filename=file.get("filename"),
                content_type=file.get("content_type"),
                chunks=[],
                embeddings=None,
            )
        )
    except Exception as error:  # noqa: BLE001  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/rag/ingestion/vertex_ai_ingestion.py#L291
        failure = str(error)
    return {
        "requests": script.requests,
        "sleep_seconds": math.fsum(script.slept),
        "token_calls": ingestion.token_calls,
        "corpus_id": corpus_id,
        "file_id": file_id,
        "failure": failure,
    }
