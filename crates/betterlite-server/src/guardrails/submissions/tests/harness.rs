#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(super) enum Failure {
    Create,
    Find,
    Name,
    TeamRows,
}

#[derive(Clone, Copy)]
enum Lookup<'text> {
    Id(&'text str),
    Name(&'text str),
}

#[derive(Default)]
struct RowStore {
    rows: BTreeMap<String, SubmissionRow>,
    created: Vec<NewSubmission>,
    team_queries: usize,
    failures: BTreeSet<Failure>,
}

#[derive(Clone, Default)]
pub(super) struct FakeRows(Arc<Mutex<RowStore>>);

#[derive(Default)]
struct TeamStore {
    teams: BTreeMap<String, Vec<String>>,
    asked: Vec<String>,
    fail: bool,
}

#[derive(Clone, Default)]
pub(super) struct FakeTeams(Arc<Mutex<TeamStore>>);

#[derive(Clone, Copy)]
pub(super) struct FixedClock(pub(super) DateTime<Utc>);

impl Clock for FixedClock {
    fn now(&self) -> DateTime<Utc> {
        self.0
    }
}

impl UserTeams for FakeTeams {
    fn team_ids(&self, user_id: &str) -> TeamIdsFuture<'_> {
        let mut store = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        store.asked.push(user_id.to_owned());
        let outcome = if store.fail {
            Err(UserLookupError::NoDatabase)
        } else {
            Ok(store.teams.get(user_id).cloned().unwrap_or_default())
        };
        drop(store);
        Box::pin(ready(outcome))
    }
}

impl FakeTeams {
    pub(super) fn allow(&self, user_id: &str, teams: &[&str]) {
        let mut store = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        store.teams.insert(
            user_id.to_owned(),
            teams.iter().map(|team| (*team).to_owned()).collect(),
        );
    }

    pub(super) fn fail(&self) {
        self.0.lock().unwrap_or_else(PoisonError::into_inner).fail = true;
    }

    pub(super) fn asked(&self) -> Vec<String> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .asked
            .clone()
    }
}

impl FakeRows {
    pub(super) fn insert(&self, row: SubmissionRow) {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .rows
            .insert(row.guardrail_id.clone(), row);
    }

    pub(super) fn created(&self) -> Vec<NewSubmission> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .created
            .clone()
    }

    pub(super) fn team_queries(&self) -> usize {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .team_queries
    }

    pub(super) fn fail(&self, failure: Failure) {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .failures
            .insert(failure);
    }

    fn lookup(
        &self,
        failure: Failure,
        query: Lookup<'_>,
    ) -> Result<Option<SubmissionRow>, SubmissionRowsError> {
        let store = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        if store.failures.contains(&failure) {
            return Err(SubmissionRowsError::MissingRow);
        }
        Ok(match query {
            Lookup::Id(id) => store.rows.get(id).cloned(),
            Lookup::Name(name) => store
                .rows
                .values()
                .find(|row| row.guardrail_name == name)
                .cloned(),
        })
    }
}

fn in_filter(row: &SubmissionRow, filter: &TeamFilter) -> bool {
    match filter {
        TeamFilter::AnyTeam => row.team_id.is_some(),
        TeamFilter::Teams(teams) => row
            .team_id
            .as_ref()
            .is_some_and(|team| teams.contains(team)),
    }
}

fn created_row(submission: &NewSubmission, index: usize) -> SubmissionRow {
    SubmissionRow {
        guardrail_id: format!("created-{index}"),
        guardrail_name: submission.guardrail_name.clone(),
        litellm_params: Some(ColumnJson::Text(submission.litellm_params.clone())),
        guardrail_info: Some(ColumnJson::Text(submission.guardrail_info.clone())),
        status: Some(submission.status.clone()),
        team_id: Some(submission.team_id.clone()),
        submitted_at: Some(Timestamp::Aware(submission.submitted_at.fixed_offset())),
        reviewed_at: None,
        created_at: Some(Timestamp::Aware(submission.created_at.fixed_offset())),
        updated_at: Some(Timestamp::Aware(submission.updated_at.fixed_offset())),
    }
}

impl SubmissionRows for FakeRows {
    fn find_by_id(
        &self,
        guardrail_id: &str,
    ) -> impl Future<Output = Result<Option<SubmissionRow>, SubmissionRowsError>> + Send {
        ready(self.lookup(Failure::Find, Lookup::Id(guardrail_id)))
    }

    fn find_by_name(
        &self,
        guardrail_name: &str,
    ) -> impl Future<Output = Result<Option<SubmissionRow>, SubmissionRowsError>> + Send {
        ready(self.lookup(Failure::Name, Lookup::Name(guardrail_name)))
    }

    fn find_team_rows(
        &self,
        filter: TeamFilter,
    ) -> impl Future<Output = Result<Vec<SubmissionRow>, SubmissionRowsError>> + Send {
        let mut store = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        store.team_queries = store.team_queries.saturating_add(1);
        if store.failures.contains(&Failure::TeamRows) {
            return ready(Err(SubmissionRowsError::MissingRow));
        }
        let rows: Vec<SubmissionRow> = store
            .rows
            .values()
            .filter(|row| in_filter(row, &filter))
            .cloned()
            .collect();
        drop(store);
        ready(Ok(rows))
    }

    fn create(
        &self,
        submission: NewSubmission,
    ) -> impl Future<Output = Result<SubmissionRow, SubmissionRowsError>> + Send {
        let mut store = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        store.created.push(submission.clone());
        if store.failures.contains(&Failure::Create) {
            return ready(Err(SubmissionRowsError::MissingRow));
        }
        let row = created_row(&submission, store.created.len());
        store.rows.insert(row.guardrail_id.clone(), row.clone());
        drop(store);
        ready(Ok(row))
    }
}

pub(super) struct Harness {
    pub(super) router: Router,
    pub(super) rows: FakeRows,
    pub(super) teams: FakeTeams,
}

pub(super) fn harness() -> Harness {
    harness_with(FakeRows::default(), FakeTeams::default())
}

pub(super) fn harness_with(rows: FakeRows, teams: FakeTeams) -> Harness {
    let application = routes(state(Some(rows.clone()), teams.clone()));
    Harness {
        router: application,
        rows,
        teams,
    }
}

pub(super) fn state(database: Option<FakeRows>, teams: FakeTeams) -> SubmissionsState<FakeRows> {
    SubmissionsState {
        database: database.map(Arc::new),
        users: Arc::new(teams),
        clock: Arc::new(FixedClock(fixed_now())),
    }
}

pub(super) fn bare_router() -> Router {
    routes(state(None, FakeTeams::default()))
}

pub(super) fn fixed_now() -> DateTime<Utc> {
    DateTime::parse_from_rfc3339("2026-01-02T03:04:05Z")
        .expect("the fixture instant")
        .with_timezone(&Utc)
}

pub(super) fn stamp(text: &str) -> Timestamp {
    Timestamp::Naive(NaiveDateTime::parse_from_str(text, "%Y-%m-%dT%H:%M:%S").expect("a stamp"))
}

pub(super) fn row(id: &str, name: &str, status: Option<&str>, team: Option<&str>) -> SubmissionRow {
    SubmissionRow {
        guardrail_id: id.to_owned(),
        guardrail_name: name.to_owned(),
        litellm_params: Some(ColumnJson::Text(REGISTER_PARAMS.to_owned())),
        guardrail_info: Some(ColumnJson::Text(
            r#"{"description":"guard check"}"#.to_owned(),
        )),
        status: status.map(str::to_owned),
        team_id: team.map(str::to_owned),
        submitted_at: Some(stamp("2026-01-02T03:04:05")),
        reviewed_at: None,
        created_at: Some(stamp("2026-01-02T03:04:05")),
        updated_at: Some(stamp("2026-01-02T03:04:05")),
    }
}

fn principal(subject: &str, roles: Vec<Role>) -> AuthContext {
    AuthContext {
        principal: Principal {
            principal_type: PrincipalType::ServiceAccount,
            subject: subject.to_owned(),
            issuer: None,
            audience: Vec::new(),
            user: None,
            organization: None,
            teams: Vec::new(),
            project: None,
            end_user: None,
            roles,
            scopes: Vec::new(),
            auth_method: AuthMethod::ApiKey,
            credential_ref: CredentialRef::default(),
            network: NetworkContext::default(),
            source_key: SourceKey::default(),
        },
    }
}

pub(super) fn admin() -> AuthContext {
    master_caller()
}

pub(super) fn role_admin() -> AuthContext {
    principal("admin-user", vec![Role::PlatformAdmin])
}

pub(super) fn caller(user_id: &str) -> AuthContext {
    let fields = [("user_id", user_id), ("user_role", "internal_user")];
    keyed(&fields)
}

pub(super) fn keyed(fields: &[(&str, &str)]) -> AuthContext {
    let mut map: BTreeMap<String, JsonData> = BTreeMap::new();
    for (name, value) in fields {
        map.insert((*name).to_owned(), JsonData::String((*value).to_owned()));
    }
    let key = UserAPIKeyAuth::construct(AuthInput::Dict(map)).expect("the key fixture constructs");
    let mut context = principal("key-owner", Vec::new());
    context.principal.source_key = SourceKey(Some(key));
    context
}

pub(super) fn with_user(
    mut context: AuthContext,
    user_id: &str,
    email: Option<&str>,
) -> AuthContext {
    context.principal.user = Some(UserIdentity {
        id: user_id.to_owned(),
        external_id: None,
        user_name: None,
        email: email.map(str::to_owned),
        display_name: None,
    });
    context
}

pub(super) fn post(uri: &str, auth: Option<AuthContext>, body: &str) -> Request<Body> {
    let mut builder = Request::builder()
        .method("POST")
        .uri(uri)
        .header(header::CONTENT_TYPE, "application/json");
    if let Some(context) = auth {
        builder = builder.extension(context);
    }
    builder
        .body(Body::from(body.to_owned()))
        .expect("a request")
}

pub(super) fn get(uri: &str, auth: Option<AuthContext>) -> Request<Body> {
    let mut builder = Request::builder().method("GET").uri(uri);
    if let Some(context) = auth {
        builder = builder.extension(context);
    }
    builder.body(Body::empty()).expect("a request")
}

pub(super) async fn send(router: &Router, request: Request<Body>) -> (StatusCode, String) {
    let response = router
        .clone()
        .oneshot(request)
        .await
        .expect("the route answers");
    let status = response.status();
    let bytes = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("the body is readable");
    (
        status,
        String::from_utf8(bytes.to_vec()).expect("the body is UTF-8"),
    )
}

pub(super) fn parse(text: &str) -> JsonData {
    serde_json::from_str(text).unwrap_or_else(|error| panic!("{text}: {error}"))
}

pub(super) fn field<'value>(value: &'value JsonData, name: &str) -> &'value JsonData {
    let JsonData::Object(fields) = value else {
        panic!("not an object: {value:?}");
    };
    fields
        .get(name)
        .unwrap_or_else(|| panic!("the body has no {name}: {value:?}"))
}

pub(super) fn text(value: &JsonData) -> &str {
    let JsonData::String(text) = value else {
        panic!("not a string: {value:?}");
    };
    text
}

pub(super) fn detail(body: &str) -> String {
    text(field(&parse(body), "detail")).to_owned()
}

pub(super) fn array(value: &JsonData) -> &[JsonData] {
    match value {
        JsonData::Array(items) => items,
        JsonData::Null
        | JsonData::Bool(_)
        | JsonData::Number(_)
        | JsonData::String(_)
        | JsonData::Object(_) => panic!("not a list: {value:?}"),
    }
}

pub(super) fn first_issue(body: &str) -> JsonData {
    let parsed = parse(body);
    let detail = field(&parsed, "detail");
    array(detail).first().cloned().expect("one issue")
}

pub(super) fn register_request(name: &str, params: &str, team: Option<&str>) -> String {
    let team_field = team.map_or_else(String::new, |chosen| format!(r#", "team_id": "{chosen}""#));
    format!(r#"{{"guardrail_name": "{name}", "litellm_params": {params}{team_field}}}"#)
}

pub(super) fn item_value(item: &GuardrailSubmissionItem) -> JsonData {
    parse(&serde_json::to_string(item).expect("the item serializes"))
}

pub(super) fn assert_error(answer: &(StatusCode, String), status: StatusCode, expected: &str) {
    assert_eq!(answer.0, status, "{}", answer.1);
    assert_eq!(detail(&answer.1), expected);
}

pub(super) fn loc_of(issue: &JsonData) -> Vec<String> {
    array(field(issue, "loc"))
        .iter()
        .map(|part| text(part).to_owned())
        .collect()
}

#[derive(Clone, Default)]
pub(super) struct Captured(Arc<Mutex<Vec<u8>>>);

impl Write for Captured {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0
            .lock()
            .map_err(|_poisoned| io::Error::other("poisoned"))?
            .extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

pub(super) fn capture() -> (Captured, DefaultGuard) {
    let captured = Captured::default();
    let writer = captured.clone();
    let subscriber = tracing_subscriber::fmt()
        .with_max_level(Level::DEBUG)
        .with_ansi(false)
        .with_writer(move || writer.clone())
        .finish();
    (captured, set_default(subscriber))
}

pub(super) fn logged(captured: &Captured) -> String {
    let bytes = captured
        .0
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .clone();
    String::from_utf8(bytes).unwrap()
}
