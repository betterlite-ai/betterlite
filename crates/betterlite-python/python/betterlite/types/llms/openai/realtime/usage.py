from typing_extensions import NotRequired, ReadOnly, TypedDict


# ast-grep-ignore: no-long-names
class OpenAIRealtimeCachedTokensDetails(TypedDict, total=False):
    text_tokens: ReadOnly[int]
    audio_tokens: ReadOnly[int]
    image_tokens: ReadOnly[int]


# ast-grep-ignore: no-long-names
class OpenAIRealtimeUsageTokenDetails(TypedDict):
    audio_tokens: ReadOnly[int]
    text_tokens: ReadOnly[int]
    cached_tokens: NotRequired[ReadOnly[int]]
    # ast-grep-ignore: no-long-names
    cached_tokens_details: NotRequired[ReadOnly[OpenAIRealtimeCachedTokensDetails]]


class OpenAIRealtimeResponseUsage(TypedDict):
    input_tokens: ReadOnly[int]
    output_tokens: ReadOnly[int]
    total_tokens: ReadOnly[int]
    # ast-grep-ignore: no-long-names
    input_token_details: NotRequired[ReadOnly[OpenAIRealtimeUsageTokenDetails]]
    # ast-grep-ignore: no-long-names
    output_token_details: NotRequired[ReadOnly[OpenAIRealtimeUsageTokenDetails]]
