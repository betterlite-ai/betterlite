use crate::panw_prisma_airs::audit::Direction;
use crate::panw_prisma_airs::construct::PanwPrismaAirs;
use crate::panw_prisma_airs::error_detail::{GuardrailHttpError, build_error_detail};
use crate::panw_prisma_airs::verdict::{Action, ScanVerdict};
use crate::prisma_airs_masking::get_masked_text;

// Allowed text takes the policy's mask whatever the flags say; blocked text only when the flag for its side is on.
pub fn masked_or_blocked_text(
    guardrail: &PanwPrismaAirs,
    verdict: &ScanVerdict,
    text: &str,
    direction: Direction,
) -> Result<String, GuardrailHttpError> {
    let masked_text = get_masked_text(&verdict.masks, direction == Direction::Response)
        .filter(|masked| !masked.is_empty());
    let content = guardrail.settings.masking.content;
    let blocked_may_mask = match direction {
        Direction::Response => content.response_content,
        Direction::Request => content.request_content,
    };
    match verdict.action {
        Action::Allow => Ok(masked_text.map_or_else(|| text.to_owned(), str::to_owned)),
        Action::Block if masked_text.is_some() && blocked_may_mask => {
            Ok(masked_text.unwrap_or_default().to_owned())
        }
        Action::Block => Err(GuardrailHttpError {
            status: 400,
            detail: Box::new(build_error_detail(guardrail, verdict, direction)),
        }),
    }
}

mod tests;
