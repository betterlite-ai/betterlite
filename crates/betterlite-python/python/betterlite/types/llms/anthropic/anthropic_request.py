from typing import ClassVar

from pydantic import ConfigDict

from betterlite.types.utils.base import ResponseObject


class AnthropicMetadata(ResponseObject):
    model_config: ClassVar[ConfigDict] = ConfigDict(extra="ignore")

    user_id: str | None = None
