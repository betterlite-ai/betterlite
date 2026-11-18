from collections.abc import Callable, Mapping
from datetime import UTC, datetime
from functools import partial
from typing import Protocol, cast

import botocore.auth  # type: ignore[import-untyped]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock/embed/embedding.py#L51-L62
from litellm.llms.bedrock.embed import embedding as embedding_module
from pydantic import BaseModel, ConfigDict, JsonValue

from .support import set_environment
from .types import Keys, SignedRequest


class KeysObject:
    def __init__(self, keys: Keys) -> None:
        self.access_key = keys["access_key_id"]
        self.secret_key = keys["secret_access_key"]
        self.token = keys.get("session_token")


class Prepared(Protocol):
    method: str
    url: str
    headers: Mapping[str, str]


class PreparedView(BaseModel):
    model_config = ConfigDict(from_attributes=True)
    method: str
    url: str
    headers: dict[str, str]


Signer = Callable[[KeysObject, str, Mapping[str, str], str], Prepared]


def signed_embedding_request(case: SignedRequest) -> JsonValue:
    set_environment({})
    botocore.auth.get_current_datetime = partial(datetime.fromtimestamp, case["now"], tz=UTC)  # type: ignore[misc]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock/embed/embedding.py#L60
    sign = cast(Signer, embedding_module._sign_get_request)  # noqa: SLF001  # pyright: ignore[reportPrivateUsage, reportUnknownMemberType]  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/bedrock/embed/embedding.py#L51-L62
    prepared = PreparedView.model_validate(
        sign(KeysObject(case["credentials"]), case["url"], case["headers"], case["region"])
    )
    return {"method": prepared.method, "url": prepared.url, "headers": prepared.headers}
