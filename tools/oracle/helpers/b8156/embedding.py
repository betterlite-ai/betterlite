import asyncio
from typing import Protocol, TypedDict, cast

import litellm
from litellm.llms.ollama.completion.handler import TokenEncoder, ollama_aembeddings  # pyright: ignore[reportUnknownVariableType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/ollama/completion/handler.py#L84
from litellm.types.utils import EmbeddingResponse
from typing_extensions import override

from .support import EmbeddingCase, EmbeddingInput, FakeClient, Json, dict_or_empty, usage_list


class WordEncoder(TokenEncoder):
    @override
    def encode(self, text: str) -> list[int]:
        return [1] * len(text.split())


class FailingEncoder(TokenEncoder):
    @override
    def encode(self, text: str) -> list[int]:
        del text
        raise ValueError("encode failed")


class EmbeddingRecord(TypedDict):
    object: str
    index: int
    embedding: Json


class EmbeddingsFunction(Protocol):
    async def __call__(  # noqa: PLR0913, PLR0917  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/ollama/completion/handler.py#L84
        self,
        api_base: str,
        model: str,
        prompts: list[str],
        model_response: EmbeddingResponse,
        optional_params: dict[str, Json],
        logging_obj: None,
        encoding: TokenEncoder | None,
    ) -> EmbeddingResponse: ...


def embedding_value(result: EmbeddingResponse) -> Json:
    records = cast("list[EmbeddingRecord]", result.data)  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/utils.py#L2302
    data: list[Json] = [
        {"object": record["object"], "index": record["index"], "embedding": record["embedding"]}
        for record in records
    ]
    return {
        "object": result.object,
        "model": result.model,
        "data": data,
        "usage": usage_list(result.usage),
        "prompt_tokens_details": None,
        "completion_tokens_details": None,
    }


async def embedding_case(case: EmbeddingCase) -> Json:
    client = FakeClient(case["status"], case["body"])
    litellm.module_level_aclient = client  # type: ignore[assignment]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/ollama/completion/handler.py#L98
    encoder_name = case.get("encoder")
    if encoder_name == "words":
        encoder: TokenEncoder | None = WordEncoder()
    elif encoder_name == "failing":
        encoder = FailingEncoder()
    else:
        encoder = None
    embeddings = cast("EmbeddingsFunction", ollama_aembeddings)  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/ollama/completion/handler.py#L84
    try:
        result = await embeddings(
            api_base=case["api_base"],
            model=case["model"],
            prompts=case["prompts"],
            model_response=EmbeddingResponse(),
            optional_params=dict_or_empty(case.get("options")),
            logging_obj=None,
            encoding=encoder,
        )
    except (AttributeError, KeyError, TypeError, ValueError):
        return {"ok": False, "status": None}
    return {
        "ok": True,
        "url": client.url,
        "payload": client.payload,
        "reply": embedding_value(result),
    }


def embedding_cases(input_value: EmbeddingInput) -> Json:
    found = [asyncio.run(embedding_case(case)) for case in input_value["cases"]]
    return {"cases": found}
