#[derive(Debug, Error)]
#[error("the permission store is down")]
struct StoreDown;

#[derive(Clone, Debug)]
enum Scripted<T> {
    Value(T),
    Fault,
    Unloadable,
}

type Reply<T> = Result<T, CeilingReadError<StoreDown>>;

impl<T: Clone> Scripted<T> {
    fn resolve(&self) -> Reply<T> {
        match self {
            Self::Value(value) => Ok(value.clone()),
            Self::Fault => Err(CeilingReadError::Failed(String::from("store down"))),
            Self::Unloadable => Err(unloadable()),
        }
    }
}

fn unloadable() -> CeilingReadError<StoreDown> {
    CeilingReadError::Unloadable(UnloadableEntitlementError::named("org", "perm-1", None))
}

#[derive(Clone, Copy, Debug)]
struct StoreState {
    database: bool,
    fresh: bool,
}

struct FakeReads {
    store: StoreState,
    managed: bool,
    marks: AdmissionMarks,
    caller: Scripted<Option<UserAPIKeyAuth>>,
    org: Scripted<Option<ObjectPermission>>,
    end_user: Scripted<Option<ObjectPermission>>,
    user: Scripted<Option<ObjectPermission>>,
    agent: Scripted<Option<ObjectPermission>>,
    team: Scripted<Option<ObjectPermission>>,
    access_group_servers: Scripted<Vec<String>>,
    team_servers: Scripted<Vec<String>>,
    agent_groups: Scripted<Option<Vec<String>>>,
    toolset: Scripted<ToolGrants>,
    toolset_server: Scripted<Option<Vec<String>>>,
    aliases: BTreeMap<String, Vec<String>>,
    log: Mutex<Vec<String>>,
}

impl FakeReads {
    fn standard() -> Self {
        Self {
            store: StoreState {
                database: true,
                fresh: false,
            },
            managed: false,
            marks: AdmissionMarks::default(),
            caller: Scripted::Value(None),
            org: Scripted::Value(None),
            end_user: Scripted::Value(None),
            user: Scripted::Value(None),
            agent: Scripted::Value(None),
            team: Scripted::Value(None),
            access_group_servers: Scripted::Value(Vec::new()),
            team_servers: Scripted::Value(Vec::new()),
            agent_groups: Scripted::Value(None),
            toolset: Scripted::Value(ToolGrants::new()),
            toolset_server: Scripted::Value(None),
            aliases: BTreeMap::new(),
            log: Mutex::new(Vec::new()),
        }
    }

    fn record(&self, entry: String) {
        self.log.lock().unwrap().push(entry);
    }

    fn calls(&self) -> Vec<String> {
        self.log.lock().unwrap().clone()
    }

    fn saw(&self, fragment: &str) -> bool {
        self.calls().iter().any(|seen| seen.contains(fragment))
    }

    fn answer<T>(&self, entry: String, script: &Scripted<T>) -> Ready<Reply<T>>
    where
        T: Clone,
    {
        self.record(entry);
        ready(script.resolve())
    }
}

impl CeilingReads for FakeReads {
    type Error = StoreDown;

    fn has_database(&self) -> bool {
        self.record(String::from("has_database"));
        self.store.database
    }

    fn admission_marks(&self, auth: &UserAPIKeyAuth) -> AdmissionMarks {
        self.record(format!("admission_marks:{}", user_id_of(Some(auth))));
        self.marks
    }

    fn requires_fresh_policy(&self, auth: Option<&UserAPIKeyAuth>) -> bool {
        self.record(format!("requires_fresh_policy:{}", user_id_of(auth)));
        self.store.fresh
    }

    async fn org_object_permission(
        &self,
        auth: Option<&UserAPIKeyAuth>,
    ) -> Reply<Option<ObjectPermission>> {
        self.answer(format!("org:{}", user_id_of(auth)), &self.org)
            .await
    }

    async fn end_user_object_permission(
        &self,
        auth: Option<&UserAPIKeyAuth>,
    ) -> Reply<Option<ObjectPermission>> {
        self.answer(format!("end_user:{}", user_id_of(auth)), &self.end_user)
            .await
    }

    async fn user_object_permission(
        &self,
        auth: Option<&UserAPIKeyAuth>,
    ) -> Reply<Option<ObjectPermission>> {
        self.answer(format!("user:{}", user_id_of(auth)), &self.user)
            .await
    }

    async fn agent_object_permission(
        &self,
        auth: Option<&UserAPIKeyAuth>,
    ) -> Reply<Option<ObjectPermission>> {
        self.answer(format!("agent:{}", user_id_of(auth)), &self.agent)
            .await
    }

    async fn team_object_permission(
        &self,
        auth: Option<&UserAPIKeyAuth>,
    ) -> Reply<Option<ObjectPermission>> {
        self.answer(format!("team:{}", user_id_of(auth)), &self.team)
            .await
    }

    async fn servers_from_access_groups(
        &self,
        access_groups: &[String],
        requires_fresh_policy: bool,
    ) -> Reply<Vec<String>> {
        self.answer(
            format!("access_groups:{access_groups:?}:{requires_fresh_policy}"),
            &self.access_group_servers,
        )
        .await
    }

    async fn team_servers(&self, auth: Option<&UserAPIKeyAuth>) -> Reply<Vec<String>> {
        self.answer(
            format!("team_servers:{}", user_id_of(auth)),
            &self.team_servers,
        )
        .await
    }

    async fn toolset_tool_permissions(
        &self,
        _permission: Option<&ObjectPermission>,
        requires_fresh_policy: bool,
    ) -> Reply<ToolGrants> {
        self.answer(format!("toolsets:{requires_fresh_policy}"), &self.toolset)
            .await
    }

    async fn toolset_tools_for_server(
        &self,
        _permission: Option<&ObjectPermission>,
        server_id: &str,
        requires_fresh_policy: bool,
    ) -> Reply<Option<Vec<String>>> {
        self.answer(
            format!("toolset_server:{server_id}:{requires_fresh_policy}"),
            &self.toolset_server,
        )
        .await
    }

    fn expand_permission_list(&self, identifiers: &[String]) -> Vec<String> {
        identifiers
            .iter()
            .flat_map(|identifier| {
                self.aliases
                    .get(identifier)
                    .cloned()
                    .unwrap_or_else(|| vec![identifier.clone()])
            })
            .collect()
    }

    fn expand_tool_permissions(&self, tool_permissions: Option<&ToolGrants>) -> ToolGrants {
        tool_permissions.cloned().unwrap_or_default()
    }

    async fn agent_access_group_server_ids(&self, agent_id: &str) -> Reply<Option<Vec<String>>> {
        self.answer(format!("agent_groups:{agent_id}"), &self.agent_groups)
            .await
    }

    async fn agent_caller_auth(&self, auth: &UserAPIKeyAuth) -> Reply<Option<UserAPIKeyAuth>> {
        self.answer(format!("caller:{}", user_id_of(Some(auth))), &self.caller)
            .await
    }

    fn is_managed_agent(&self, auth: Option<&UserAPIKeyAuth>) -> bool {
        self.record(format!("managed:{}", user_id_of(auth)));
        self.managed
    }
}

fn user_id_of(auth: Option<&UserAPIKeyAuth>) -> String {
    let Some(caller) = auth else {
        return String::from("none");
    };
    decode::<StoreDown>(Some(caller))
        .ok()
        .and_then(|record| record.user_id)
        .unwrap_or_default()
}

fn auth_from_json(json: &str) -> UserAPIKeyAuth {
    let fields: BTreeMap<String, JsonData> = serde_json::from_str(json).unwrap();
    UserAPIKeyAuth::construct(AuthInput::Dict(fields)).unwrap()
}

fn org_auth() -> UserAPIKeyAuth {
    auth_from_json(r#"{"org_id":"org-1"}"#)
}

fn user_auth() -> UserAPIKeyAuth {
    auth_from_json(r#"{"user_id":"user-1"}"#)
}

fn end_user_auth() -> UserAPIKeyAuth {
    auth_from_json(r#"{"user_id":"user-1","end_user_id":"eu-1"}"#)
}

fn agent_auth() -> UserAPIKeyAuth {
    auth_from_json(r#"{"agent_id":"agent-1","org_id":"org-1"}"#)
}

fn admin_auth() -> UserAPIKeyAuth {
    auth_from_json(r#"{"user_id":"admin-1","user_role":"proxy_admin"}"#)
}

fn opaque_span_auth() -> UserAPIKeyAuth {
    let caller = AgentCaller {
        user_id: Some(String::from("user-1")),
        team_id: Some(String::from("team-1")),
    };
    caller_record(&caller, false, Some(ParentOtelSpan::Opaque)).unwrap()
}

fn names(items: &[&str]) -> Vec<String> {
    items.iter().map(|item| String::from(*item)).collect()
}

fn row(object_permission_id: &str) -> ObjectPermission {
    ObjectPermission {
        object_permission_id: String::from(object_permission_id),
        mcp_servers: PermissionList(Some(Vec::new())),
        mcp_access_groups: PermissionList(Some(Vec::new())),
        mcp_tool_permissions: None,
        vector_stores: PermissionList(Some(Vec::new())),
        agents: PermissionList(Some(Vec::new())),
        agent_access_groups: PermissionList(Some(Vec::new())),
        models: PermissionList(Some(Vec::new())),
        mcp_toolsets: None,
        blocked_tools: PermissionList(Some(Vec::new())),
        search_tools: PermissionList(Some(Vec::new())),
        mcp_tool_search_enabled: None,
        skills: None,
    }
}

fn with_servers(mut permission: ObjectPermission, servers: &[&str]) -> ObjectPermission {
    permission.mcp_servers = PermissionList(Some(names(servers)));
    permission
}

fn with_groups(mut permission: ObjectPermission, groups: &[&str]) -> ObjectPermission {
    permission.mcp_access_groups = PermissionList(Some(names(groups)));
    permission
}

fn with_tools(
    mut permission: ObjectPermission,
    server_id: &str,
    tools: &[&str],
) -> ObjectPermission {
    permission.mcp_tool_permissions =
        Some(BTreeMap::from([(String::from(server_id), names(tools))]));
    permission
}

fn without_tools(mut permission: ObjectPermission) -> ObjectPermission {
    permission.mcp_tool_permissions = Some(BTreeMap::new());
    permission
}

fn capped(servers: &[&str], restricts: bool) -> Capped {
    Capped {
        servers: names(servers),
        restricts,
    }
}

fn tool_grants(entries: &[(&str, &[&str])]) -> ToolGrants {
    entries
        .iter()
        .map(|(server_id, tools)| (String::from(*server_id), names(tools)))
        .collect()
}

fn over(reads: FakeReads) -> Ceilings<FakeReads> {
    Ceilings { reads }
}
