pub mod to_ir;
pub mod to_systemone;

macro_rules! decision_response_conversion {
    (
        $fn_name:ident,
        $response:ident: $Source:ty,
        $request:ident,
        $Output:path,
        $keys:ident,
        answers: $answers:expr,
        usage: $usage:expr
        $(, $named_field:ident: $named_value:expr)*
    ) => {
        #[must_use]
        pub fn $fn_name($response: &$Source, $request: &ir::DecisionRequest) -> $Output {
            let $keys =
                $crate::decisions::transformation::questions::systemone_keys(&$request.questions);
            $Output {
                model: $response.model.clone(),
                answers: $answers,
                usage: $usage,
                $($named_field: $named_value,)*
                extra: $response.extra.clone(),
            }
        }
    };
}
pub(crate) use decision_response_conversion;
