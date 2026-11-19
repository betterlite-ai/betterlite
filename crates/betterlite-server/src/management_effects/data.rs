#[derive(Clone, Copy, Debug, PartialEq, Eq)]
// Each pair of variants is one LiteLLM request class: an update of one id and a delete of many
pub enum ManagementData<'data> {
    UpdateKey(&'data str),
    DeleteKeys(&'data [String]),
    UpdateUser(&'data str),
    DeleteUsers(&'data [String]),
    UpdateTeam(&'data str),
    DeleteTeams(&'data [String]),
    UpdateCustomer(&'data str),
    DeleteCustomers(&'data [String]),
}
