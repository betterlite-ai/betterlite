import asyncio
import base64
import importlib
from functools import partial
from typing import Literal, NotRequired, TypeAlias, TypedDict

import httpx
import litellm
from litellm.litellm_core_utils import url_utils
from litellm.llms.base_llm.ocr.transformation import OCRPage, OCRResponse
from litellm.models.credentials import CredentialItem
from litellm.rag.ingestion import base_ingestion
from litellm.rag.ingestion.base_ingestion import BaseRAGIngestion
from litellm.types.rag import (
    RAGChunkingStrategy,
    RAGIngestEmbeddingOptions,
    RAGIngestOCROptions,
    RAGIngestOptions,
)
from pydantic import JsonValue
from typing_extensions import override


class CredentialInput(TypedDict):
    name: str
    values: dict[str, JsonValue]


class FileDataInput(TypedDict):
    filename: str
    content_base64: str
    content_type: str


class FetchInput(TypedDict):
    status: int
    content_type: NotRequired[str]
    body_base64: str


class InitInput(TypedDict):
    mode: Literal["init"]
    options: RAGIngestOptions
    credentials: NotRequired[list[CredentialInput]]


class UploadInput(TypedDict):
    mode: Literal["upload"]
    options: RAGIngestOptions
    file_data: NotRequired[FileDataInput]
    file_url: NotRequired[str]
    file_id: NotRequired[str]
    fetch: NotRequired[FetchInput]


class OcrInput(TypedDict):
    mode: Literal["ocr"]
    options: RAGIngestOptions
    file_content_base64: NotRequired[str]
    content_type: NotRequired[str]
    pages: NotRequired[list[str]]


class ChunkInput(TypedDict):
    mode: Literal["chunk"]
    options: RAGIngestOptions
    text: NotRequired[str]
    file_content_base64: NotRequired[str]
    ocr_was_used: NotRequired[bool]


Input: TypeAlias = InitInput | UploadInput | OcrInput | ChunkInput


class IngestIdView(TypedDict):
    prefixed: bool
    suffix_len: int


class InitView(TypedDict):
    ingest_id: IngestIdView
    ingest_name: str | None
    ocr_config: dict[str, str] | None
    chunking_strategy: dict[str, JsonValue]
    embedding_config: dict[str, str] | None
    vector_store_config: dict[str, JsonValue]
    supports_existing_file_id: bool


class UploadView(TypedDict):
    filename: str | None
    file_content_base64: str | None
    content_type: str | None
    existing_file_id: str | None


class OcrCallView(TypedDict):
    model: str
    document: dict[str, JsonValue]


class OcrView(TypedDict):
    text: str | None
    calls: list[OcrCallView]


class ChunkView(TypedDict):
    chunks: list[str]


class TestIngestion(BaseRAGIngestion):
    @override
    async def store(  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/rag/ingestion/base_ingestion.py#L269-L294
        self,
        file_content: bytes | None,
        filename: str | None,
        content_type: str | None,
        chunks: list[str],
        embeddings: list[list[float]] | None,
        existing_file_id: str | None = None,
    ) -> tuple[str | None, str | None]:
        del file_content, filename, content_type, chunks, embeddings, existing_file_id
        return None, None

    @override
    def chunk(
        self,
        text: str | None,
        file_content: bytes | None,
        ocr_was_used: bool,
    ) -> list[str]:
        strategy = self.ingest_options.get("chunking_strategy")
        if strategy is not None:
            _check_chunking(strategy)
        return super().chunk(text, file_content, ocr_was_used)


def _check_chunking(strategy: RAGChunkingStrategy) -> None:
    size = strategy.get("chunk_size")
    overlap = strategy.get("chunk_overlap")
    if size is not None and size <= 0:
        raise ValueError("chunk_size must be greater than zero")
    if overlap is not None and overlap < 0:
        raise ValueError("chunk_overlap must not be negative")
    if size is not None and overlap is not None and overlap >= size:
        raise ValueError("chunk_overlap must be less than chunk_size")


class OcrRecorder:
    def __init__(self, pages: list[str]) -> None:
        self.pages = pages
        self.calls: list[OcrCallView] = []

    async def aocr(self, model: str, document: dict[str, JsonValue]) -> OCRResponse:
        self.calls.append({"model": model, "document": dict(document)})
        pages = [
            OCRPage(index=index, markdown=text) for index, text in enumerate(self.pages)
        ]
        return OCRResponse(model=model, pages=pages)


class FetchAnswer:
    def __init__(self, fetch: FetchInput) -> None:
        self._status = fetch["status"]
        self._body = base64.b64decode(fetch["body_base64"])
        self._headers: dict[str, str] = {}
        content_type = fetch.get("content_type")
        if content_type is not None:
            self._headers["content-type"] = content_type

    async def answer(self, client: httpx.AsyncClient, url: str) -> httpx.Response:
        del client
        reply = httpx.Response(self._status, headers=self._headers, content=self._body)
        reply.request = httpx.Request("GET", url)
        return reply


def run(input_value: Input) -> InitView | UploadView | OcrView | ChunkView:
    if input_value["mode"] == "init":
        return _init(input_value)
    if input_value["mode"] == "upload":
        return _upload(input_value)
    if input_value["mode"] == "ocr":
        return _ocr(input_value)
    return _chunk(input_value)


def _build(
    options: RAGIngestOptions, credentials: list[CredentialInput] | None
) -> TestIngestion:
    info: dict[str, str] = {"source": "b3451"}
    listed = (
        [
            CredentialItem(
                credential_name=credential["name"],
                credential_info=info,
                credential_values=credential["values"],
            )
            for credential in credentials
        ]
        if credentials is not None
        else []
    )
    litellm.credential_list = listed
    return TestIngestion(options)


def _option_config(
    options: RAGIngestOptions,
) -> tuple[dict[str, str] | None, dict[str, str] | None]:
    ocr = options.get("ocr")
    embedding = options.get("embedding")
    return _model_config(ocr), _model_config(embedding)


def _model_config(
    config: RAGIngestOCROptions | RAGIngestEmbeddingOptions | None,
) -> dict[str, str] | None:
    if config is None:
        return None
    model = config.get("model")
    if model is None:
        return {}
    return {"model": model}


def _init(input_value: InitInput) -> InitView:
    credentials = input_value.get("credentials")
    instance = _build(input_value["options"], credentials)
    ocr_config, embedding_config = _option_config(input_value["options"])
    identifier = instance.ingest_id
    ingest_id = IngestIdView(
        prefixed=identifier.startswith("ingest_"),
        suffix_len=len(identifier) - len("ingest_"),
    )
    return InitView(
        ingest_id=ingest_id,
        ingest_name=input_value["options"].get("name"),
        ocr_config=ocr_config,
        chunking_strategy=instance.chunking_strategy,
        embedding_config=embedding_config,
        vector_store_config=instance.vector_store_config,
        supports_existing_file_id=instance.supports_existing_file_id,
    )


def _upload(input_value: UploadInput) -> UploadView:
    fetch = input_value.get("fetch")
    if fetch is not None:
        url_utils.async_safe_get = partial(FetchAnswer(fetch).answer)
        importlib.reload(base_ingestion)
    instance = _build(input_value["options"], None)
    file_data_option = input_value.get("file_data")
    if file_data_option is None:
        file_data = None
    else:
        file_data = (
            file_data_option["filename"],
            base64.b64decode(file_data_option["content_base64"]),
            file_data_option["content_type"],
        )
    filename, content, content_type, existing_file_id = asyncio.run(
        instance.upload(
            file_data=file_data,
            file_url=input_value.get("file_url"),
            file_id=input_value.get("file_id"),
        )
    )
    return UploadView(
        filename=filename,
        file_content_base64=None
        if content is None
        else base64.b64encode(content).decode("ascii"),
        content_type=content_type,
        existing_file_id=existing_file_id,
    )


def _ocr(input_value: OcrInput) -> OcrView:
    instance = _build(input_value["options"], None)
    pages = input_value.get("pages")
    recorder = OcrRecorder([] if pages is None else pages)
    litellm.aocr = partial(recorder.aocr)
    encoded = input_value.get("file_content_base64")
    content = None if encoded is None else base64.b64decode(encoded)
    text = asyncio.run(instance.ocr(content, input_value.get("content_type")))
    return OcrView(text=text, calls=recorder.calls)


def _chunk(input_value: ChunkInput) -> ChunkView:
    instance = _build(input_value["options"], None)
    encoded = input_value.get("file_content_base64")
    content = None if encoded is None else base64.b64decode(encoded)
    ocr_option = input_value.get("ocr_was_used")
    ocr_was_used = False if ocr_option is None else ocr_option
    chunks = instance.chunk(input_value.get("text"), content, ocr_was_used)
    return ChunkView(chunks=chunks)
