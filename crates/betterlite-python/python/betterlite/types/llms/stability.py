from typing import Final, Literal

from typing_extensions import TypedDict


# ast-grep-ignore: no-long-names
class StabilityImageGenerationRequest(TypedDict, total=False):
    prompt: str
    negative_prompt: str | None
    aspect_ratio: str | None
    seed: int | None
    output_format: Literal["jpeg", "png", "webp"] | None
    model: str | None
    mode: Literal["text-to-image", "image-to-image"] | None
    image: str | None
    strength: float | None
    style_preset: str | None


# ast-grep-ignore: no-long-names
class StabilityImageEditRequest(StabilityImageGenerationRequest):
    mask: str | None


# ast-grep-ignore: no-long-names
class StabilityImageGenerationResponse(TypedDict, total=False):
    image: str
    finish_reason: str
    seed: int


class StabilityUpscaleRequest(TypedDict, total=False):
    image: str
    prompt: str | None
    negative_prompt: str | None
    output_format: Literal["jpeg", "png", "webp"] | None
    seed: int | None
    creativity: float | None


class StabilityInpaintRequest(TypedDict, total=False):
    image: str
    prompt: str
    mask: str | None
    negative_prompt: str | None
    seed: int | None
    output_format: Literal["jpeg", "png", "webp"] | None
    grow_mask: int | None


class StabilityOutpaintRequest(TypedDict, total=False):
    image: str
    prompt: str | None
    negative_prompt: str | None
    left: int | None
    right: int | None
    up: int | None
    down: int | None
    seed: int | None
    output_format: Literal["jpeg", "png", "webp"] | None
    creativity: float | None


class StabilityEraseRequest(TypedDict, total=False):
    image: str
    mask: str | None
    seed: int | None
    output_format: Literal["jpeg", "png", "webp"] | None
    grow_mask: int | None


class StabilitySearchReplaceRequest(TypedDict, total=False):
    image: str
    prompt: str
    search_prompt: str
    negative_prompt: str | None
    seed: int | None
    output_format: Literal["jpeg", "png", "webp"] | None
    grow_mask: int | None


# ast-grep-ignore: no-long-names
class StabilityRemoveBackgroundRequest(TypedDict, total=False):
    image: str
    output_format: Literal["png", "webp"] | None


class StabilityControlRequest(TypedDict, total=False):
    image: str
    prompt: str
    negative_prompt: str | None
    control_strength: float | None
    seed: int | None
    output_format: Literal["jpeg", "png", "webp"] | None


class StabilityEditResponse(TypedDict, total=False):
    image: str
    finish_reason: str
    seed: int


# ast-grep-ignore: no-long-names
OPENAI_SIZE_TO_STABILITY_ASPECT_RATIO: Final = {
    "1024x1024": "1:1",
    "1792x1024": "16:9",
    "1024x1792": "9:16",
    "512x512": "1:1",
    "256x256": "1:1",
}

STABILITY_ASPECT_RATIOS: Final = [
    "1:1",
    "16:9",
    "9:16",
    "4:3",
    "3:4",
    "21:9",
    "9:21",
    "3:2",
    "2:3",
    "5:4",
    "4:5",
]

STABILITY_GENERATION_MODELS: Final = {
    "sd3": "/v2beta/stable-image/generate/sd3",
    "sd3.5-large": "/v2beta/stable-image/generate/sd3",
    "sd3.5-large-turbo": "/v2beta/stable-image/generate/sd3",
    "sd3.5-medium": "/v2beta/stable-image/generate/sd3",
    "sd3-large": "/v2beta/stable-image/generate/sd3",
    "sd3-large-turbo": "/v2beta/stable-image/generate/sd3",
    "sd3-medium": "/v2beta/stable-image/generate/sd3",
    "stable-image-ultra": "/v2beta/stable-image/generate/ultra",
    "stable-image-core": "/v2beta/stable-image/generate/core",
}

STABILITY_EDIT_ENDPOINTS: Final = {
    "inpaint": "/v2beta/stable-image/edit/inpaint",
    "outpaint": "/v2beta/stable-image/edit/outpaint",
    "erase": "/v2beta/stable-image/edit/erase",
    "search-and-replace": "/v2beta/stable-image/edit/search-and-replace",
    "search-and-recolor": "/v2beta/stable-image/edit/search-and-recolor",
    "remove-background": "/v2beta/stable-image/edit/remove-background",
    "replace-background-and-relight": "/v2beta/stable-image/edit/replace-background-and-relight",
    "fast": "/v2beta/stable-image/upscale/fast",
    "conservative": "/v2beta/stable-image/upscale/conservative",
    "creative": "/v2beta/stable-image/upscale/creative",
    "sketch": "/v2beta/stable-image/control/sketch",
    "structure": "/v2beta/stable-image/control/structure",
    "style": "/v2beta/stable-image/control/style",
    "style-transfer": "/v2beta/stable-image/control/style-transfer",
}
