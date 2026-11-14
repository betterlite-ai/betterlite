use rust_decimal::Decimal;

pub struct UserBudgetEmail<'text> {
    pub logo_url: &'text str,
    pub user_name: Option<&'text str>,
    pub max_budget: Option<Decimal>,
    pub support_contact: &'text str,
}

pub struct TeamBudgetEmail<'text> {
    pub logo_url: &'text str,
    pub team_alias: Option<&'text str>,
    pub max_budget: Option<Decimal>,
    pub spend: Decimal,
    pub support_contact: &'text str,
}

#[must_use]
pub fn render_user_budget_email(email: &UserBudgetEmail<'_>) -> String {
    let logo_url = email.logo_url;
    let user_name = email.user_name.unwrap_or("None");
    let max_budget = email
        .max_budget
        .map_or_else(|| "None".to_owned(), |budget| budget.to_string());
    let support_contact = email.support_contact;
    format!(
        "\n            <img src=\"{logo_url}\" alt=\"LiteLLM Logo\" width=\"150\" height=\"50\" />\n\n            <p> Hi {user_name}, <br/>\n\n            Your LLM API usage this month has reached your account's <b> monthly budget of ${max_budget} </b> <br /> <br />\n\n            API requests will be rejected until either (a) you increase your monthly budget or (b) your monthly usage resets at the beginning of the next calendar month. <br /> <br />\n\n            If you have any questions, please send an email to {support_contact} <br /> <br />\n\n            Best, <br />\n            The LiteLLM team <br />\n            "
    )
}

#[must_use]
pub fn render_team_budget_email(email: &TeamBudgetEmail<'_>) -> String {
    format!(
        concat!(
            "\n    <img src=\"{}\" alt=\"LiteLLM Logo\" width=\"150\" height=\"50\" /> <br/><br/><br/>\n\n",
            "    Budget Crossed for Team <b> {} </b> <br/> <br/>\n\n",
            "    Your Teams LLM API usage has crossed it's <b> budget of ${} </b>, current spend is <b>${}</b><br /> <br />\n\n",
            "    API requests will be rejected until either (a) you increase your budget or (b) your budget gets reset <br /> <br />\n\n",
            "    If you have any questions, please send an email to {} <br /> <br />\n\n",
            "    Best, <br />\n    The LiteLLM team <br />\n    "
        ),
        email.logo_url,
        email.team_alias.unwrap_or("None"),
        email
            .max_budget
            .map_or_else(|| "None".to_owned(), |budget| budget.to_string()),
        email.spend,
        email.support_contact
    )
}
