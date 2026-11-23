use crate::catalog_queries::CatalogQueries;
use crate::catalog_queries::error::CatalogQueryError;
use crate::deployment_catalog::scope::litellm_model;
use crate::deployment_model_info::text;

impl CatalogQueries<'_> {
    // Router.is_recognized_model: a deployment name, id or alias, or a callable routing group.
    #[must_use]
    pub fn is_recognized_model(&self, model: &str) -> bool {
        self.lookup.catalog.model_names.contains(model)
            || self.lookup.catalog.id_index.contains_key(model)
            || self.lookup.model_group_alias.contains_key(model)
            || self.lookup.routing_groups.callable_group(model).is_some()
    }

    // Router.map_team_model: the first row the team owns gives the caller's name, or names its deployment.
    pub fn map_team_model(
        &self,
        team_model_name: Option<&str>,
        team_id: &str,
    ) -> Result<Option<String>, CatalogQueryError> {
        let rows = self.lookup.get_model_list(team_model_name, Some(team_id))?;
        let Some(row) = rows
            .iter()
            .find(|found| text(&found.model_info, "team_id") == Some(team_id))
        else {
            return Ok(None);
        };
        if let Some(name) = team_model_name {
            return Ok(Some(name.to_owned()));
        }
        let public =
            text(&row.model_info, "team_public_model_name").filter(|name| !name.is_empty());
        Ok(Some(
            public.map_or_else(|| row.model_name.clone(), str::to_owned),
        ))
    }

    // Router.resolved_litellm_models: the provider model of every row the name reaches, in listing order.
    pub fn resolved_litellm_models(
        &self,
        model_name: &str,
        team_id: Option<&str>,
    ) -> Result<Vec<String>, CatalogQueryError> {
        let mut models = Vec::new();
        for row in self.lookup.get_model_list(Some(model_name), team_id)? {
            let provider_model = litellm_model(&row);
            if !provider_model.is_empty() {
                models.push(provider_model.to_owned());
            }
        }
        Ok(models)
    }

    // Router._is_model_access_group_for_wildcard_route: a wildcard group holds a model the pattern router matches.
    pub fn is_wildcard_access_group(&self, group: &str) -> Result<bool, CatalogQueryError> {
        let declared =
            self.access_groups
                .get_model_access_groups(&self.lookup, None, Some(group), None)?;
        let Some(models) = declared.get(group) else {
            return Ok(false);
        };
        Ok(models.iter().any(|listed| {
            self.lookup
                .catalog
                .pattern_router
                .route(listed, None)
                .is_some()
        }))
    }
}
