from enum import StrEnum
from typing import Final, Literal, Protocol

VERTEX_CREDENTIALS_TYPES = str | dict[str, str]


class VertexAccessTokenResolver(Protocol):
    async def __call__(
        self,
        credentials: VERTEX_CREDENTIALS_TYPES | None,
        project_id: str | None,
        custom_llm_provider: Literal["vertex_ai", "vertex_ai_beta", "gemini"],
    ) -> tuple[str, str]: ...


class VertexPartnerProvider(StrEnum):
    mistralai = "mistralai"
    llama = "llama"
    ai21 = "ai21"
    claude = "claude"


# ast-grep-ignore: no-long-names
VERTEX_AI_PROVIDER_METADATA_FIELDS: Final = (
    "vertex_ai_grounding_metadata",
    "vertex_ai_url_context_metadata",
    "vertex_ai_safety_ratings",
    "vertex_ai_safety_results",
    "vertex_ai_citation_metadata",
)
