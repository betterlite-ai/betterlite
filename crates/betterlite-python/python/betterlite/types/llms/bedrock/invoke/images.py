from typing import Literal

from pydantic import JsonValue
from typing_extensions import Required, TypedDict


# ast-grep-ignore: no-long-names
class AmazonStability3TextToImageRequest(TypedDict, total=False):
    prompt: str
    aspect_ratio: Literal["16:9", "1:1", "21:9", "2:3", "3:2", "4:5", "5:4", "9:16", "9:21"]
    mode: Literal["image-to-image", "text-to-image"]
    output_format: Literal["JPEG", "PNG"]
    seed: int
    negative_prompt: str


# ast-grep-ignore: no-long-names
class AmazonStability3TextToImageResponse(TypedDict, total=False):
    images: list[str]
    seeds: list[str]
    finish_reasons: list[str]


class AmazonTitanTextToImageParams(TypedDict, total=False):
    text: Required[str]
    negativeText: str


class AmazonNovaCanvasRequestBase(TypedDict, total=False):
    pass


# ast-grep-ignore: no-long-names
class AmazonNovaCanvasImageGenerationConfig(TypedDict, total=False):
    cfgScale: int
    seed: int
    quality: Literal["standard", "premium"]
    width: int
    height: int
    numberOfImages: int


class AmazonNovaCanvasTextToImageParams(TypedDict, total=False):  # ast-grep-ignore: no-long-names
    text: str
    negativeText: str
    controlStrength: float
    controlMode: Literal["CANNY_EDIT", "SEGMENTATION"]
    conditionImage: str


# ast-grep-ignore: no-long-names
class AmazonNovaCanvasTextToImageRequest(AmazonNovaCanvasRequestBase, TypedDict, total=False):
    # ast-grep-ignore: no-long-names
    textToImageParams: AmazonNovaCanvasTextToImageParams
    taskType: Literal["TEXT_IMAGE"]
    # ast-grep-ignore: no-long-names
    imageGenerationConfig: AmazonNovaCanvasImageGenerationConfig


# ast-grep-ignore: no-long-names
class AmazonNovaCanvasColorGuidedGenerationParams(TypedDict, total=False):
    colors: list[str]
    referenceImage: str
    text: str
    negativeText: str


# ast-grep-ignore: no-long-names
class AmazonNovaCanvasColorGuidedRequest(AmazonNovaCanvasRequestBase, TypedDict, total=False):
    taskType: Literal["COLOR_GUIDED_GENERATION"]
    # ast-grep-ignore: no-long-names
    colorGuidedGenerationParams: AmazonNovaCanvasColorGuidedGenerationParams
    # ast-grep-ignore: no-long-names
    imageGenerationConfig: AmazonNovaCanvasImageGenerationConfig


class AmazonNovaCanvasTextToImageResponse(TypedDict, total=False):  # ast-grep-ignore: no-long-names
    images: list[str]


class AmazonNovaCanvasInpaintingParams(TypedDict, total=False):  # ast-grep-ignore: no-long-names
    text: str
    maskImage: str
    inputImage: str
    negativeText: str


# ast-grep-ignore: no-long-names
class AmazonNovaCanvasInpaintingRequest(AmazonNovaCanvasRequestBase, TypedDict, total=False):
    taskType: Literal["INPAINTING"]
    # ast-grep-ignore: no-long-names
    inpaintingParams: AmazonNovaCanvasInpaintingParams
    # ast-grep-ignore: no-long-names
    imageGenerationConfig: AmazonNovaCanvasImageGenerationConfig


# ast-grep-ignore: no-long-names
class AmazonTitanImageGenerationRequestBody(TypedDict, total=False):
    taskType: Literal["TEXT_IMAGE", "COLOR_GUIDED_GENERATION", "INPAINTING"]
    textToImageParams: AmazonTitanTextToImageParams
    # ast-grep-ignore: no-long-names
    imageGenerationConfig: AmazonNovaCanvasImageGenerationConfig


class BedrockPreparedRequest(TypedDict):
    endpoint_url: str
    prepped: JsonValue
    body: bytes
    data: dict[str, JsonValue]
