use std::future::Future;

use betterlite_prompts::arguments::json::Object;

use crate::dcr_hydration::{DcrClientStore, DcrHydration};
use crate::openapi_tools::tool::OpenApiToolConfig;
use crate::server_record::MCPServer;

pub trait DcrHydrator: Send + Sync {
    fn hydrate(&self, server: &mut MCPServer) -> impl Future<Output = bool> + Send;
}

impl<Store> DcrHydrator for DcrHydration<Store>
where
    Store: DcrClientStore,
{
    async fn hydrate(&self, server: &mut MCPServer) -> bool {
        self.hydrate_config_server_dcr_client(server).await
    }
}

pub trait OpenApiToolSink: Send + Sync {
    fn register_tool(
        &self,
        name: &str,
        description: &str,
        input_schema: Object,
        config: OpenApiToolConfig,
    );
    fn set_tool_owner(&self, tool_name: &str, server_prefix: &str);
    fn rebuild_tool_name_mapping(&self);
}

pub trait InstructionsCache: Send + Sync {
    fn clear(&self);
}

pub trait ServerDefinitionCaches: Send + Sync {
    fn invalidate(&self, server_id: &str);
}
