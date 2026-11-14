from typing_extensions import TypedDict


class AimlImageSize(TypedDict, total=False):
    width: int
    height: int


# ast-grep-ignore: no-long-names
class AimlImageGenerationRequestParams(TypedDict, total=False):
    model: str
    prompt: str
    image_size: AimlImageSize | str
    safety_tolerance: str | None
    output_format: str | None
    num_images: int | None
    seed: int | None
    enable_safety_checker: bool | None
