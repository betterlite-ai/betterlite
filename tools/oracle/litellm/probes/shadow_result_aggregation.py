from litellm.proxy.management_endpoints.auto_router_endpoints import _pct_of
from typing_extensions import override

from probes.protocol import Operation, Strict

# pyright: strict, reportAny=true, reportExplicitAny=true, reportImplicitOverride=true, reportPrivateUsage=false
# _pct_of is private and no public entry reaches the rounding rule on its own
# https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/litellm/proxy/management_endpoints/auto_router_endpoints.py#L1331-L1332


class Pair(Strict):
    numerator: int
    denominator: int


class Output(Strict):
    rates: list[float]


class ShadowResultAggregation(Operation):
    pairs: list[Pair]

    @override
    def run(self) -> Output:
        return Output(rates=[_pct_of(pair.numerator, pair.denominator) for pair in self.pairs])
