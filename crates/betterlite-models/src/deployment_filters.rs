mod tests;

pub trait OrderedDeployment {
    fn litellm_params_order(&self) -> Option<i64>;

    fn model_info_order(&self) -> Option<i64>;

    // model_info only counts for deployments added through the API, where order never sits in litellm_params
    fn order(&self) -> Option<i64> {
        self.litellm_params_order()
            .or_else(|| self.model_info_order())
    }
}

#[must_use]
pub fn get_order_filtered_deployments<Deployment>(
    healthy_deployments: Vec<Deployment>,
    target_order: Option<i64>,
) -> Vec<Deployment>
where
    Deployment: OrderedDeployment,
{
    let wanted = target_order.or_else(|| {
        healthy_deployments
            .iter()
            .filter_map(OrderedDeployment::order)
            .min()
    });
    let Some(order) = wanted else {
        return healthy_deployments;
    };
    healthy_deployments
        .into_iter()
        .filter(|deployment| deployment.order() == Some(order))
        .collect()
}
