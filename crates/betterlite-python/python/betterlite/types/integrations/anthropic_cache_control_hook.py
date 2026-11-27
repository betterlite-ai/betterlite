from typing import Final, Literal

from typing_extensions import NotRequired, ReadOnly, TypedDict

# ast-grep-ignore: no-long-names
GATEWAY_INJECTED_CACHE_METADATA_KEY: Final = "litellm_gateway_injected_cache"
# ast-grep-ignore: no-long-names
GATEWAY_INJECTED_FOR_EVERY_DEPLOYMENT: Final = ""


# LiteLLM imports this from litellm.types.llms.openai; betterlite has no such module.
class ChatCompletionCachedContent(TypedDict):
    type: Literal["ephemeral"]
    ttl: NotRequired[Literal["5m", "1h"]]


# ast-grep-ignore: no-long-names
class CacheControlMessageInjectionPoint(TypedDict):
    location: Literal["message"]
    role: Literal["user", "system", "assistant"] | None
    index: int | str | None
    control: ChatCompletionCachedContent | None
    _litellm_openai_dialect: NotRequired[ReadOnly[bool]]
    _litellm_external_breakpoints: NotRequired[ReadOnly[int]]


# ast-grep-ignore: no-long-names
class CacheControlToolConfigInjectionPoint(TypedDict):
    location: Literal["tool_config"]
    control: ChatCompletionCachedContent | None
    _litellm_openai_dialect: NotRequired[ReadOnly[bool]]
    _litellm_external_breakpoints: NotRequired[ReadOnly[int]]


CacheControlInjectionPoint = (
    # ast-grep-ignore: no-long-names
    CacheControlMessageInjectionPoint | CacheControlToolConfigInjectionPoint
)
