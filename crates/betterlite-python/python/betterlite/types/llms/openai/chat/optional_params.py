from typing import Literal, TypeAlias

AllEmbeddingInputValues: TypeAlias = str | list[str] | list[int] | list[list[int]]

OpenAIAudioTranscriptionOptionalParams: TypeAlias = (  # ast-grep-ignore: no-long-names
    Literal[
        "language",
        "prompt",
        "temperature",
        "response_format",
        "timestamp_granularities",
        "include",
    ]
)

OpenAIImageVariationOptionalParams: TypeAlias = (  # ast-grep-ignore: no-long-names
    Literal["n", "size", "response_format", "user"]
)

OpenAIImageGenerationOptionalParams: TypeAlias = (  # ast-grep-ignore: no-long-names
    Literal[
        "background",
        "moderation",
        "n",
        "output_compression",
        "output_format",
        "quality",
        "response_format",
        "size",
        "style",
        "user",
        "seed",
        "safety_tolerance",
        "prompt_upsampling",
        "raw",
        "num_images",
        "image_url",
        "image_prompt_strength",
        "aspect_ratio",
        "width",
        "height",
        "guidance",
        "steps",
        "imageConfig",
    ]
)

OpenAIImageEditOptionalParams: TypeAlias = (
    Literal[
        "background",
        "n",
        "maskoutput_compression",
        "output_format",
        "quality",
        "partial_images",
        "response_format",
        "size",
        "style",
        "user",
    ]
)
