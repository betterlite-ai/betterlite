import asyncio
import warnings
from typing import Final
from unittest.mock import patch

from litellm.constants import DEFAULT_AUTO_ROUTER_MAX_INPUT_CHARS
from litellm.router import Router
from litellm.router_strategy.auto_router.auto_router import AutoRouter
from litellm.types.utils import Embedding, EmbeddingResponse
from pydantic import Field, JsonValue
from typing_extensions import override

from probes.protocol import Operation, Strict

# pyright: strict, reportAny=true, reportExplicitAny=true, reportImplicitOverride=true, reportMissingTypeStubs=false

MODEL_NAME: Final = "semantic-router"


class MessageInput(Strict):
    role: str
    content: JsonValue | None = None


class DocumentArgs(Strict):
    model: str
    input: list[str]


class QueryArgs(Strict):
    model: str
    input: list[str]
    metadata: dict[str, JsonValue]
    litellm_metadata: dict[str, JsonValue]
    proxy_server_request: dict[str, JsonValue]
    turn_off_message_logging: bool | None = None
    litellm_session_id: str | None = None
    litellm_trace_id: str | None = None


class DocumentCall(Strict):
    model: str
    input: list[str]


class QueryCall(Strict):
    model: str
    input: list[str]
    metadata: dict[str, JsonValue]
    litellm_metadata: dict[str, JsonValue]
    proxy_server_request: dict[str, JsonValue]
    turn_off_message_logging: bool | None
    litellm_session_id: str | None
    litellm_trace_id: str | None


class Round(Strict):
    failed: bool
    model: str | None
    documents: list[DocumentCall]
    queries: list[QueryCall]


class Output(Strict):
    accepted: bool
    code: str | None
    rounds: list[Round]


class Provider:
    def __init__(self, payload: "SemanticRouter") -> None:
        self.vectors: dict[str, list[float]] = payload.vectors
        self.embedding_model: str = payload.embedding_model
        self.document_failures: int = payload.document_failures
        self.query_failures: int = payload.query_failures
        self.documents: list[DocumentCall] = []
        self.queries: list[QueryCall] = []

    def response(self, model: str, inputs: list[str]) -> EmbeddingResponse:
        data = [
            Embedding(embedding=self.vectors[text], index=index, object="embedding")
            for index, text in enumerate(inputs)
        ]
        return EmbeddingResponse(model=model, data=data)

    def embedding(self, *_args: object, **kwargs: object) -> EmbeddingResponse:
        args = DocumentArgs.model_validate(kwargs)
        self.documents.append(DocumentCall(model=args.model, input=args.input))
        if self.document_failures > 0:
            self.document_failures -= 1
            raise ValueError("the probe failed the document embedding")
        return self.response(args.model, args.input)

    async def aembedding(self, *_args: object, **kwargs: object) -> EmbeddingResponse:
        args = QueryArgs.model_validate(kwargs)
        self.queries.append(
            QueryCall(
                model=args.model,
                input=args.input,
                metadata=args.metadata,
                litellm_metadata=args.litellm_metadata,
                proxy_server_request=args.proxy_server_request,
                turn_off_message_logging=args.turn_off_message_logging,
                litellm_session_id=args.litellm_session_id,
                litellm_trace_id=args.litellm_trace_id,
            )
        )
        if self.query_failures > 0:
            self.query_failures -= 1
            raise ValueError("the probe failed the query embedding")
        return self.response(args.model, args.input)


class SemanticRouter(Operation):
    config: str
    default_model: str
    embedding_model: str
    max_input_chars: int | None = None
    messages: list[MessageInput] = Field(default_factory=list)
    metadata: JsonValue | None = None
    litellm_metadata: JsonValue | None = None
    litellm_params_metadata: JsonValue | None = None
    litellm_session_id: JsonValue | None = None
    litellm_trace_id: JsonValue | None = None
    turn_off_message_logging: JsonValue | None = None
    vectors: dict[str, list[float]] = Field(default_factory=dict)
    document_failures: int = 0
    query_failures: int = 0
    rounds: int = 1
    concurrent: int = 1
    pad_bytes: int = 0

    @override
    def run(self) -> Output:
        provider = Provider(self)
        # semantic_router's route sync calls pydantic's deprecated dict(); that is not compared behavior
        warnings.filterwarnings(
            "ignore",
            message="The `dict` method is deprecated",
            category=DeprecationWarning,
        )
        with (
            patch.object(Router, "embedding", provider.embedding),
            patch.object(Router, "aembedding", provider.aembedding),
        ):
            return asyncio.run(self.rounds_output(provider))

    def auto_router(self) -> AutoRouter:
        limit = (
            DEFAULT_AUTO_ROUTER_MAX_INPUT_CHARS
            if self.max_input_chars is None
            else self.max_input_chars
        )
        return AutoRouter(
            model_name=MODEL_NAME,
            default_model=self.default_model,
            embedding_model=self.embedding_model,
            litellm_router_instance=Router(model_list=[]),
            auto_router_config=self.config + " " * self.pad_bytes,
            max_input_chars=limit,
        )

    async def rounds_output(self, provider: Provider) -> Output:
        router = self.auto_router()
        request_kwargs = self.request_kwargs()
        messages: list[dict[str, object]] = [
            {"role": message.role, "content": message.content} for message in self.messages
        ]
        rounds: list[Round] = []
        for _round in range(self.rounds):
            documents = len(provider.documents)
            queries = len(provider.queries)
            tasks: list[asyncio.Task[tuple[bool, str | None]]] = [
                asyncio.ensure_future(self.call(router, messages, request_kwargs))
                for _caller in range(self.concurrent)
            ]
            outcomes = [await task for task in tasks]
            failed = any(not ok for ok, _model in outcomes)
            model = next((model for _ok, model in outcomes if model is not None), None)
            rounds.append(
                Round(
                    failed=failed,
                    model=None if failed else model,
                    documents=provider.documents[documents:],
                    queries=provider.queries[queries:],
                )
            )
        return Output(accepted=True, code=None, rounds=rounds)

    async def call(
        self,
        router: AutoRouter,
        messages: list[dict[str, object]],
        request_kwargs: dict[str, object],
    ) -> tuple[bool, str | None]:
        try:
            response = await router.async_pre_routing_hook(
                model=MODEL_NAME,
                request_kwargs=request_kwargs,
                messages=messages,
            )
        except ValueError:
            return False, None
        return True, None if response is None else response.model

    def request_kwargs(self) -> dict[str, object]:
        kwargs: dict[str, object] = {}
        if self.metadata is not None:
            kwargs["metadata"] = self.metadata
        if self.litellm_metadata is not None:
            kwargs["litellm_metadata"] = self.litellm_metadata
        if self.litellm_params_metadata is not None:
            kwargs["litellm_params"] = {"metadata": self.litellm_params_metadata}
        if self.turn_off_message_logging is not None:
            kwargs["turn_off_message_logging"] = self.turn_off_message_logging
        if self.litellm_session_id is not None:
            kwargs["litellm_session_id"] = self.litellm_session_id
        if self.litellm_trace_id is not None:
            kwargs["litellm_trace_id"] = self.litellm_trace_id
        return kwargs
