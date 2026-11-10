mod tests;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BudgetScope {
    Key,
    User,
    EndUser,
    Team,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct BudgetPrefixes {
    pub(crate) spend: &'static str,
    pub(crate) start_time: &'static str,
}

impl BudgetScope {
    pub(crate) const fn prefixes(self) -> BudgetPrefixes {
        match self {
            Self::Key => BudgetPrefixes {
                spend: "virtual_key_spend",
                start_time: "virtual_key_budget_start_time",
            },
            Self::User => BudgetPrefixes {
                spend: "user_model_spend",
                start_time: "user_model_budget_start_time",
            },
            Self::EndUser => BudgetPrefixes {
                spend: "end_user_model_spend",
                start_time: "end_user_budget_start_time",
            },
            Self::Team => BudgetPrefixes {
                spend: "team_model_spend",
                start_time: "team_model_budget_start_time",
            },
        }
    }
}
