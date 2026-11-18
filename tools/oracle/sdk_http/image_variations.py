from typing import ClassVar

from not_built import Unbuilt


class ImageVariationsRequest(Unbuilt):
    task: ClassVar[str] = "B1000"

    image_variations: None
