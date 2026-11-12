import time
import uuid
from typing import TypeAlias, cast
from uuid import UUID

import litellm

litellm.suppress_debug_info = True

Json: TypeAlias = None | bool | int | float | str | list["Json"] | dict[str, "Json"]
Object: TypeAlias = dict[str, Json]


class FixedUuid:
    def __call__(self) -> UUID:
        return UUID(int=0)


class FixedClock:
    def __call__(self) -> int:
        return 7


uuid.uuid4 = FixedUuid()  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/utils.py#L123
time.time = FixedClock()  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/types/utils.py#L2135


def as_object(value: Json) -> Object:
    return cast(Object, value)  # https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/llms/openai/chat/gpt_transformation.py#L864-L866
