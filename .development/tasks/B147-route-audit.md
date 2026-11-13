# B147 page request audit

Pin: `d665dc1a6315847fcad6cc2c1264ceb47c06abce` (LiteLLM 1.105.1), read from the sibling checkout without enterprise. B9090 takes in **all** these sources unchanged before reviewed patches; the counts below are client contracts, not a React-to-Rust port. `GET` requests have no body. JSON writes use the discovered auth header (default `Authorization: Bearer <session key>`), unless the row identifies a bug. An omitted optional field is not `null`. Paths below are proxy paths, before `proxy_base_url`/root composition. No additional aliases are requested by these pages.

Coverage terms: **planned** means a task lists an HTTP case, not that that file exists or has passed. The current `tools/oracle/cases/models/list-shared-name.yaml` already sends the bare GET `/models` that Lens sends. Searches found no existing HTTP cases for the other complete feature-page requests below; matching daily-activity SQL, routing probes, ROI GitHub and Lens helper cases are not HTTP evidence. Dependencies can materialize their planned cases before these tasks run; reuse an exact case if that happens. The tables distinguish already-planned equivalent requests from the missing combinations this batch adds.

## Logs and traces

Read [request construction](https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/ui/litellm-dashboard/src/components/networking.tsx#L1876-L2030), [filter composition](https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/ui/litellm-dashboard/src/components/view_logs/log_filter_logic.tsx#L143-L207), [deep link](https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/ui/litellm-dashboard/src/components/view_logs/RequestLogsPanel.tsx#L111-L158), and [session paging](https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/ui/litellm-dashboard/src/components/view_logs/LogDetailsDrawer/LogDetailsDrawer.tsx#L115-L166).

| Caller / method and path | Query or JSON body | Fields consumed | Route owner; HTTP evidence / remainder |
| --- | --- | --- | --- |
| RequestLogsPanel GET `/spend/logs/ui` | UTC `start_date,end_date` as `YYYY-MM-DD HH:mm:ss`, `page=1,page_size=25,sort_by=startTime,sort_order=desc,group_by_session=true`; optional `api_key,team_id,request_id,search,session_id,user_id,end_user,status_filter,cache_hit_filter,used_client_oauth_token,span_type,model_id,model,key_alias,error_code,error_message,session_cursor,exclude_internal_health_checks` | `data,total,page,page_size,total_pages,has_more,next_session_cursor`; row ids, times, model/group, key alias, user/team/end-user, session, status, tokens, spend, metadata and latency | B1732; B8663 mounts and plans `http/spend-log-list/*`, including grouping/filter/cursor boundaries. B9741 adds the actual initial combined query and row-to-detail handoff. Networking's default 50 is overridden by the component's 25. Networking also accepts numeric `min_spend,max_spend`, but this filter component does not send them. |
| RequestLogsPanel / useSpanRequestLog GET same | deep link uses log window; span lookup uses span start +/-30 minutes, both UTC `YYYY-MM-DD HH:mm:ss`; `page=1,page_size=1,request_id=<selected>` | selected `data` row | B1732; B9741 exact deep link and span window. |
| useLogDetails GET `/spend/logs/ui/{request_id}` | `start_date=<row startTime>` | log payload, messages, response, metadata, guardrail information | B1729; B8663 `http/spend-log-payload/*` already plans collision/authorization. B9741 follows the list's id **and date**, without inventing `start_time`. |
| LogDetailsDrawer GET `/spend/logs/session/ui` | `session_id,page=1,page_size=100`, subsequent page numbers | `data,total_pages`, lean chronological rows | B1731; B8663 `http/spend-session/page-size-100-lean.yaml` and visibility case planned; only combined drawer handoff is new. |
| LogFilters GET `/management/v1/spend_logs/users` and `/management/v1/spend_logs/end_users` | `filter[startTime][gte]`, `filter[startTime][lte]`, `page=1,page_size=50`, optional `q`; next page from links | `data`, `links.next` and facet values | B1705; `http/management-spend-facets/*` already covers exact paging/bounds/search. Reuse, no second facet implementation or generic case. |
| CompliancePanel POST `/compliance/eu-ai-act` and `/compliance/gdpr` | `{request_id,user_id,model,timestamp,guardrail_information}` from selected row/metadata | `compliant,regulation,checks[].{check_name,article,passed,detail}` | B1654 already plans this complete body and success/failure HTTP cases. B9741 browser/component proof only. |
| AuditLogsPanel GET `/audit` | `page=1,page_size=50,sort_by=updated_at,sort_order=desc`; optional `search,object_id,changed_by,object_key_hash,object_team_id,action,table_name` | `audit_logs,total,page,page_size,total_pages`; before/after values, actor, action, table, timestamps | **No OSS handler**. Whole-checkout search finds the auth route name but no decorator; B065 is audit delivery, not this listing. B9741 proves authenticated 404 and replaces the enterprise advertisement with unavailable text. No enterprise implementation. |
| useAgentTraces GET `/v1/traces` | range `start_ms,end_ms` epoch milliseconds, optional opaque `cursor`; setup probe sends only `start_ms=0` | `data:TraceSummary[],next_cursor` | B3867/B110; seven disabled-route HTTP cases planned, but not the range/probe/cursor request forms. B9741 adds these with tracing absent. |
| TracingSetupCard POST `/v1/traces` | its generated OTLP JSON `resourceSpans` sample | success then trace polling; error `detail` on 501 | B3867/B110; disabled POST already planned. Do not claim working ingest/storage from an error test. |
| TraceDrawer GET `/v1/traces/{trace_id}` | optional `trace_ref` | `summary,spans` and trace metadata | B3867/B110; disabled detail planned; B9741 adds encoded reference query. |
| DetailContent GET `/v1/traces/{trace_id}/spans/{span_id}` | optional `trace_ref` | span attributes, inputs, outputs, events and error information | B3867/B110; same distinction. |
| DetailContent GET `/v1/traces/{trace_id}/spans/{span_id}/error` | `trace_ref`, optional opaque `cursor` | error-page content and `next_cursor` | B3867/B110; same distinction. |
| TracingSetupCard POST `/key/generate` | `{key_alias:"Agent tracing",allowed_routes:["/v1/traces"],metadata:{purpose:"agent_tracing"}}` | `key` (the secret) | B055; B9741 adds this exact restricted-key request. `/v1/traces/query` and `/query/help` are backend APIs but have no caller in these page components. |

DeletedKeysPage/DeletedTeamsPage are mounted tabs of `view_logs/index.tsx`; their list and mutations belong to B9730/B9731 under B145, not this batch. Selector requests shared with keys/teams/models are enumerated below.

## Usage and exports

[networking daily family](https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/ui/litellm-dashboard/src/components/networking.tsx#L1298-L1401) is called by `UsagePageView`, `EntityUsage`, `TopKeyView`, `EntityUsageExport`, and the cost-optimization range hooks. `activity_metrics` transforms responses; it has no independent request. `view_user_spend` calls the models selector even though part of its display is commented out.

For **each** E in `user,team,tag,organization,customer,agent`, the common query is `start_date,end_date` as YYYY-MM-DD, browser `timezone` in minutes west of UTC, optional `model,api_key`, and entity selector: respectively `user_id` (first selected only), `team_ids,tags,organization_ids,end_user_ids,agent_ids` (comma-separated). Optional exclusions are `exclude_team_ids,exclude_organization_ids,exclude_end_user_ids,exclude_agent_ids`; team defaults exclude `litellm-dashboard`. Cost optimization adds `include_current_utc_day=true` for user only. Ordinary Usage does not. Empty arrays and missing optional fields are omitted.

| Method/path family | Extra query | Fields consumed | Owner and coverage |
| --- | --- | --- | --- |
| GET `/{E}/daily/activity/aggregated` | optional `api_key_limit` | `results[].date,metrics,breakdown`; `metadata` total spend/flat cost, requests, success/failure, tokens, cache reads/writes, compression/caching/autorouter savings, response time/timed requests and pagination | B3097/B8482; generic empty/default/bounds cases planned. B9742 adds six page-specific filtered requests and nonempty user/team/tag data. |
| GET `/{E}/daily/activity/aggregated/keys` | `offset,limit` | key rows, metrics, metadata (`key_alias,team_id,user_id,user_email,key_exists`), paging | B3096/B8482; generic offset/alias cases planned, B9742 adds page filter continuation. |
| GET `/{E}/daily/activity/aggregated/search` | `search`, optional `limit` | matched keys and metadata | B3097/B8482; generic search cases planned, B9742 combines entity/exclusion/timezone/search. |
| GET `/{E}/daily/activity/aggregated/model_top_keys` | `model_group,by_model_group=true|false`, optional `limit` | top key rows and metrics | B3096/B8482; defaults planned, B9742 follows actual model/model-group selection. |
| GET `/{E}/daily/activity/export` | `export_type=daily|daily_with_keys|daily_with_models|daily_with_users`, `format=csv|json` (only choices enabled by the entity modal) | blob; Content-Type/Content-Disposition, CSV headings/rows or JSON values | B3098/B8482; empty/default export cases planned. B9742 adds enabled modal choices with the common page query and populated available tables. |
| GET `/user/daily/activity/aggregated/cache_leakage_keys` | common user query with current-day flag; page omits limit | keys and estimated uncached/lost-savings metrics | B3096/B8482; default case planned. B9743 proves the cost page's date/timezone/current-day request. |
| GET `/team/spend/by_user` | dates, `team_ids` comma-separated | teams and member spend/token/request totals | B815; B9742 adds actual TeamUserSpendCard query. |
| GET `/gateway/daily/activity` | dates | gateway daily categories and totals | B3102/B8661; default/custom-window HTTP planned; B9742 reuses or supplies exact selected window. |
| GET `/v1/tool/spend` | dates | per-tool and daily spend/request entries | B1607; B9743 adds UsageTab request. |
| GET `/tag/distinct` | none | `results[].tag` | B3100/B8661 planned exact request; reuse. |
| GET `/tag/dau`, `/tag/wau`, `/tag/mau` | `end_date`; optional `tag_filter` or **repeated** `tag_filters` array | `results[]` tag/date/period and active-user counts | B3100 (dau/wau), B3101 (mau), B8661 mount; B9742 adds the two-selected-tags forms. |
| GET `/tag/summary` | dates, repeated `tag_filters` | tag summary results | B3101/B8661; B9742 filtered page request. |
| GET `/tag/user-agent/per-user-analytics` | `page=1,page_size=50`, repeated `tag_filters`; **no dates** | users, email, agent, usage and pagination | B3101/B8661; B9742 exact filtered pagination. |
| POST `/usage/ai/chat` | `{messages:[{role,content}],model}`; selected model from `/model_group/info` | SSE `chunk.content,status.message,tool_call,done,error.message` | B8271/B3103/B8673; B074's newly written `http/usage-chat/*` plans exact model/history, tool and error streams. **No duplicate in B147.** |

Breakdown maps consumed by the retained adapters are `models,model_groups,mcp_servers,providers,api_keys,entities,endpoints`; nested metrics, metadata and `api_key_breakdown` must not be dropped from comparisons. Export download is a real HTTP blob; ROI CSV and CloudZero's placeholder CSV button are not export endpoints.

Legacy `old-usage/_components/usage.tsx` uses [these wrappers](https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/ui/litellm-dashboard/src/components/networking.tsx#L2031-L2183):

| Method/path | Query/body; consumed fields | Owner; remainder |
| --- | --- | --- |
| GET `/global/spend/logs` | no query; daily spend rows | B1724/B8662; B9742 page-load flow |
| GET `/global/spend/keys`, `/global/spend/models` | `limit=5`; key/alias or model and total_spend | B1724/B8662; B9742 exact limit |
| POST `/global/spend/end_users` | initial `{}`, filtered `{api_key?,startTime:<ISO>,endTime:<ISO>}`; end_user,total_spend | B1722/B8662; B9742 omission versus selected interval |
| GET `/global/spend/teams` | no query; daily_spend and teams | B1724/B8662; B9742 page-load flow |
| GET `/global/spend/tags` | ISO dates, optional comma-separated tags; spend_per_tag | B1723/B8662; B9742 selected tags |
| GET `/global/spend/all_tag_names` | none; tag_names | B1723/B8662; generic request already planned |
| GET `/global/spend/provider` | dates; provider spend | B1721/B8662; B9742 selected window |
| GET `/global/activity`, `/global/activity/model` | dates; daily_data, api_requests,total_tokens, sums; per-model groups | B1721/B8662; B9742 activity tab |
| GET `/sso/get/ui_settings` | none; DISABLE_EXPENSIVE_DB_QUERIES controls which calls run | B1627/B8770; B144 already covers, no duplicate |

## Cost tracking and optimization

| Caller / method/path | Query/body and fields read | Owner; evidence / remainder |
| --- | --- | --- |
| discount/margin hooks GET, PATCH `/config/cost_discount_config`, `/config/cost_margin_config` | PATCH raw provider map, discount numbers; margin numbers or `{percentage,fixed_amount}` including global; GET/PATCH read `values` | B1650; route HTTP cases planned, B9743 adds edited forms followed by readback/estimate |
| block hook GET, PATCH `/config/block_requests_for_models_without_pricing` | PATCH `{enabled:<bool>}`; read enabled | B1650; same composition |
| pricing calculator POST `/cost/estimate` | `{model,input_tokens,output_tokens,num_requests_per_day,num_requests_per_month}`; token defaults 0 and unused request counts **null** | B1651; B9743 page-null-count shape; reads provider, token prices, per-request/input/output/margin costs and daily/monthly amounts |
| CloudZero settings GET `/cloudzero/settings` | no query; api_key_masked,connection_id,timezone | B3076/B073; B073 CRUD composition planned |
| POST `/cloudzero/init`, PUT `/cloudzero/settings` | `{connection_id,timezone,api_key?}`; settings form defaults UTC and omits empty key on update | B3076/B073; B9743 tests omitted-key update and response reload |
| DELETE `/cloudzero/delete` | no body; success message then settings reload | B3076/B073; already planned |
| POST `/cloudzero/dry-run` | `{limit:10}`; entire preview JSON | B3076/B073; B9743 exact preview limit |
| POST `/cloudzero/export` | settings card `{operation:"replace_hourly"}`; usage modal `{limit:100000,operation:"replace_hourly"}` | B3076/B073; B9743 both forms on empty export (no external delivery) |
| CloudZero modal GET/settings, POST/init, PUT/update, POST/export | same paths; modal init/update always supplies api_key,connection_id,timezone UTC | Existing modal hardcodes root-relative fetch; B9743 patches to apiClient and treats null connection/key settings as unconfigured, preserving custom header and root. |
| benchmark hook GET `/auto_router/benchmarks` | YYYY-MM-DD dates; end becomes UTC today only when the range ends locally today and UTC today is later; optional api_key,user_id | B355/B387; routing probes only. B9743 HTTP; reads groups/totals, router type/name, savings, tier_turns and cache same_model/first_visit/return_to_tier turns/hit_rate_pct |
| shadow hooks GET `/auto_router/shadow_eval`, GET `/{job_id}` | no query | B357; routing probes only, B9743 HTTP; jobs, arms/slices/cost, status, target, deadlines |
| POST `/auto_router/shadow_eval/start` | api_key_ids,team_ids,user_ids,models,router_names arrays; direction forward/reverse; reverse baseline_model; shadow_percentage,duration_days,max_budget,judge_model | B356; B9743 forward and reverse form bodies, then list/detail |
| POST `/auto_router/shadow_eval/{job_id}/stop` | **no body** | B356; B9743 stop/readback |
| requests table GET `/cost_optimization/prompt_caching/requests` | ISO start at T00:00:00.000Z and end at T23:59:59.999Z after benchmarksWindow, filter all/injected/hits, page_size=10 (25/50/100 choices), paired cursor_start_time/cursor_request_id | B3125/B066; only errors currently planned. B9743 success/empty and paired-cursor requests; reads requests,has_more,next_cursor and row request/time/model/cache-token/savings fields |
| PromptCachingTab GET `/config/list` | `config_type=general_settings`; array field_name,field_value,field_description,field_options,stored_in_db | B8595/B8675; generic list planned |
| embedded PromptCachingPanel POST `/config/field/update`, `/config/field/delete` | `{config_type:"general_settings",field_name,field_value}` (delete omits value); fields enable_anthropic_prompt_caching,anthropic_prompt_caching_ttl,openai_system_messages_first | B8595, B8675 mount. B9743 exact true/1h/reset form and subsequent list, external component retained by B146 |
| PromptCompressionTab GET `/v2/guardrails/list`, fallback GET `/guardrails/list` after any failed v2 fetch/status/JSON parse | guardrails[].guardrail_id/name,litellm_params.guardrail/api_base/default_on | B3684/B8667; existing route tests do not prove fallback component; component check plus B9743 page HTTP |
| POST `/guardrails` | `{guardrail_name:<trimmed>,litellm_params:{guardrail:"headroom",mode:"pre_call",api_base:<trimmed>,default_on:true}}` | B3685; B9743 exact form then list; no actual Headroom request |

## Lens and ROI

Read [LensView](https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/ui/litellm-dashboard/src/app/(dashboard)/lens/_components/LensView.tsx#L80-L243), [analysis key](https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/ui/litellm-dashboard/src/app/(dashboard)/lens/_components/AnalysisKey.tsx#L30-L62), [key creation](https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/ui/litellm-dashboard/src/app/(dashboard)/lens/_components/AnalysisKey.tsx#L170-L185), and [ROI view](https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/ui/litellm-dashboard/src/app/(dashboard)/roi-calculator/_components/ROICalculatorView.tsx#L63-L202).

| Method/path | Page query/body and fields consumed | Owner; coverage |
| --- | --- | --- |
| GET `/lens` | lenses,workers,tracing_enabled; ids/settings/jobs/findings/cost/last_scan_at | B4503/B8682; B9030 exact empty/list/history flow planned |
| GET `/lens/activity/available`, `/lens/agents` | no query; traces/requests flags, agent names | B4503/B8682; B9030 exact disabled responses planned |
| POST `/lens/preview/sample` | `{offset:0,as_of:<ISO>,settings:{source,service:"",filters:[],execution_ids:[],name:"Preview",model:"preview",checks:[{id:"preview",instruction:"Preview recorded activity"}],team_id?,agent_name?,sample_percent?,sample_size?},lookback_hours:24}` | B4503; B9744 exact ActivityScope body; sample executions/eligibility if available, but current binding returns 501 |
| POST `/lens`, PUT `/lens/{id}` | full form Settings: name,model,context,source,service,filters,scope/sampling,execution_ids,enabled,schedule/concurrency/budget defaults as lensData/Setup produce | B4503/B8682; B9030 partial settings/history planned. B9744 complete serialized Settings, no fabricated worker/storage success |
| POST `/lens/{id}/runs`, `/lens/{id}/cancel` | `{}`; invalidate list | B4503; B9030 already plans these exact bodies |
| GET `/lens/{id}/runs`, `/lens/{id}/runs/{job_id}` | list offset=0 then 50; Job[] / Job incl settings,revision,sample,findings,status,usage | B4503; first page planned B9030, B9744 next-page and full-form handoff |
| PATCH `/lens/{id}/findings/{finding_id}` | `{status,reason}` | B4503; B9030 dismissed/missing case planned; do not seed fake successful analysis |
| GET `/lens/{id}/executions/{encoded execution_id}` | offset=0 then next; ExecutionContent and paging | B4503; B9744 disabled-storage response with actual encoded page path |
| POST `/lens/workers/register` | `{name:"Lens worker",analysis_key_id:<hash>}`; worker and one-time token | B4503; B9030 omits name, B9744 exact form after actual key generation |
| PUT `/lens/workers/{worker_id}/billing-key`, DELETE `/lens/workers/{worker_id}` | PUT `{analysis_key_id}`, DELETE no body | B4503; B9030 exact route bodies planned; new key-to-worker composition B9744 |
| GET `/key/list` | page=1,size=25,return_full_object=true,substring_matching=true,include_team_keys=true,include_created_by_keys=true,status=active,optional key_alias | B782; B9744 exact Lens selector (different from B9730 table) |
| GET `/key/info`, POST `/key/generate`, POST `/key/delete` | info key=<hash>; generate `{key_alias:"Lens analysis",models:[model],max_budget:<number>,budget_duration:"1mo",metadata:{purpose:"lens"}}`; cleanup `{keys:[new hash]}` only after failed registration and confirmed unreferenced worker | B778/B055/B777; B9744 generation/token_id/register composition; B9730 already owns generic info/delete bodies |
| GET/PUT `/roi-calculator/settings` | PUT github_api_url,repos,estimator_model,estimator_prompt,backfill_days,update_interval_minutes; unchanged secrets omitted, clear=null, new=trimmed string | B3158/B8677; generic get/put/validation planned; B9744 complete form omission/clear/readback |
| GET `/roi-calculator/repositories` | onboarding no query, search `query=<text including empty>,page=1` | B3161/B8677; query and default planned; repositories,has_more |
| POST `/roi-calculator/connections/test` | no body; success or displayed error | B3161/B8677; already planned |
| GET `/roi-calculator/report` | no query or `mode=demo` | B8588/B8677; live empty planned, B9744 demo (summary,metrics,people,pulls,effort_basis) |
| GET/POST/DELETE `/roi-calculator/sync` | **no body for POST or DELETE** | B8588/B8677; status and start-not-ready planned, B9744 idle cancel/status; running,phase,elapsed/started/finished,counts,error |
| PUT `/roi-calculator/identity-map` | `{github_login,email:<string or null>}` | B8588/B8677; only empty-login error planned, B9744 add/remove; identity_map and report |
| POST `/roi-calculator/setup/reset` | no body | B8588/B8677; only viewer403 planned, B9744 admin settings/reset readback |

Lens evidence traces reuse the trace routes above. B8682 deliberately binds storage=None even with CLICKHOUSE_URL; B110 has the same native storage gap. B147 cannot turn those decisions into functional ClickHouse investigations. B9744 retains the disabled state. WorkerSetup's pinned `ghcr.io/berriai/litellm-lens-worker` command must become the B8232 local betterlite image instructions, not an invented public image. ROI's people CSV is generated in the browser, with no proxy CSV route.

## Shared selectors and retained code outside page call graphs

| Request and consumer | Data read | Existing owner / follow-up |
| --- | --- | --- |
| GET `/key/info?key=<hash>` (log/usage detail) | info, token, alias, budget/model/owner fields | B778; B9730 exact generic request planned |
| GET `/key/aliases` (log filter), page=1,size=50,optional search/team_id | aliases[] strings,current_page,total_pages,total_count,size | B782; B9741 exact selector query |
| GET `/v2/model/info` (log filter), include_team_models=true,page=1,size=50,optional search | data[].model_info.id,model_name and total_pages | B850; B9741 exact selector query |
| GET `/v2/model/info` (cost benchmark/shadow catalog), include_team_models=true,page=1,size=1000, then remaining pages | data[].model_name,litellm_params.model and router configs,model_info.id/mode/team_id/db_model | B850; B9743 exact catalog request |
| Shadow selectors GET `/key/list`, `/user/list`, `/v2/team/list` | keys page=1,size=50,four true flags,optional key_alias (no expand/sort); users page=1,page_size=50,optional user_email; teams page=1,page_size=20,optional team_alias/user_id; consume key token/alias, user id/email, team id/alias and pagination | B782/B796/B813; B9743 page-shaped selectors, reuse B9730/B9731/B9732 if exact |
| GET `/v2/team/list?page=1&page_size=100`, optional user_id; GET `/team/list` (log/usage selectors) | teams, ids/aliases, membership/budget, total_pages | B813; B9731 page selector cases planned |
| GET `/user/list` page=1,page_size=1,user_ids=<selected>; table lookup sends sorted distinct comma-separated user_ids and page_size=min(count,100) | users[].user_id,user_email and pagination | B796; B9732 management list planned; B9741 selected-user and batch lookups |
| GET `/v2/user/info` with current-user/default query | user fields spend,max_budget | B791; B9732 planned |
| GET `/organization/list`, `/customer/list`, `/v1/agents`, `/tag/list?start_date&end_date` | organization ids/alias; customer id; agents ids/names; tags object names | B1212, B059, B4784, B1604 respectively; B9733 organization selectors and B9735 resources planned; B9742 dated tag query |
| GET `/models?include_model_access_groups=True`, optional team_id (view_user_spend/fetch_models); GET `/models` (Lens) | data[].id | B007; B9735 common flagged model requests; bare Lens request already exists in models/list-shared-name.yaml and B9744 reuses it |
| GET `/model_group/info` (calculator/AI chat/Lens model pickers) | data[].model_group,mode,prices,context and supported parameters | B060; B9735 exact request planned. B849 owns deployment search, not this handler. |

The `llm_calls` data-plane functions have callers in B146's playground/chat, not in the listed B147 page flows: chat completion, Responses, MCP/code-interpreter event transforms and request-header helpers are retained by B9090, not reimplemented or given duplicate B147 HTTP cases. `fetch_models` is reachable here and stays in B147's final component proof. B146 owns the external playground browser workflow. Shell discovery/login/logout/settings belong to B144.

## Trace evidence and boundaries

The audit ran `cargo gate trace` against handlers as well as grepping task descriptions for literal paths. Locations below are in `litellm/proxy/`, at this pin. Where a decorator-only trace had no hit, the function-body trace and literal route grep resolved the owner; that is not unowned work.

* `spend_tracking/spend_management_endpoints.py`: 650/792/1277 -> B1721; 2083/2134 -> B1723; 2458/2466 and 2766 -> B1732; 3385 -> B1729; 3840/4022/4069/4274 -> B1724; 4190 -> B1722; 4505 -> B1731. Mounts B8662/B8663.
* `management_endpoints/management_v1/spend_logs.py:172,223` -> B1705; `compliance_endpoints.py:26,55` -> B1654.
* `tracing_endpoints.py:97,123,185,201,214,227,241` -> B3867, B110 composition. The separate Anthropic gateway decorator is not a second owner for these dashboard requests.
* `management_endpoints/daily_activity_routes.py:273,345,394,444,496,549` -> B3097/B3096/B3098; B8482 mounts. `management_endpoints/user_agent_analytics_endpoints.py:162,213,305,426,547,652` -> B3100/B3101; `management_endpoints/gateway_request_endpoints.py:94` -> B3102; B8661 mounts. `management_endpoints/usage_endpoints/endpoints.py:34` -> B8271, B8673 mounts, B074 HTTP.
* `management_endpoints/cost_tracking_settings.py:192,227,325,360,507,517,563` -> B1650/B1651; `auto_router_endpoints.py:846,1580,1802,1849,1897` -> B355/B356/B357; `prompt_caching_requests.py:128` -> B3125.
* `spend_tracking/cloudzero_endpoints.py:133,192,345,395,453,518` -> B3076, B073 composition.
* `lens/endpoints.py:146,156,172,178,184,203,215,220,229,238,257,283,305,321,336,557` -> B4503, B8682 mount, B9030 database HTTP. B4505's worker-facing protocol is not called by the browser.
* `management_endpoints/roi_calculator_endpoints.py:320,332,391,419,436,465,481,504,599,620` -> B3158/B3161/B8588, B8677 mount. Broad old B076/B3158 references do not transfer B8588 ownership back to the umbrella.
* `management_endpoints/key_management_endpoints.py:1940,4069,4346,6486,6721` -> B055/B777/B778/B782; `management_endpoints/team_endpoints.py:5591,5857,6748` -> B813/B815; `management_endpoints/internal_user_endpoints.py:1067,2218` -> B791/B796; `management_endpoints/organization_endpoints.py:1082` -> B1212; `management_endpoints/customer_endpoints.py:837` -> B059; `agent_endpoints/endpoints.py:378` -> B4784 (read; B4783 overlaps broad references); `management_endpoints/tag_management_endpoints.py:575` -> B1604; `management_endpoints/tool_management_endpoints.py:233` -> B1607.
* `proxy_server.py:11549,15295,16359,18259,18728,18905` -> B007/B850/B060/B8595; `guardrails/guardrail_endpoints.py:134,187,335` -> B3684/B3685.

The supplemental trace run was repeated after B9091's transient YAML error was corrected. Facets, benchmark, Lens runs and spend tag/list handler traces resolved as above. `/models` and config traces still return no owner because B007 and B8595 do not include proxy_server.py in covers; literal route/task references establish their ownership. `/model_group/info` at 16359 falls in a gap between B060's references, despite its description explicitly owning 16359-16581. Architect corrections: add proxy_server.py to B007/B8595 covers, extend B060's second reference to start at decorator 16354, and extend B355's reference start from 846 to decorator 840. These are provenance corrections, not new route assignments; none was edited here.

Response-envelope details used by the client adapters: daily key pages read `api_keys,offset,limit,total_api_keys`; search and cache-leakage responses read `api_keys`; model-top-keys reads `api_keys,model,by_model_group`. Team/member spend reads `results,start_date,end_date`; tool spend reads `by_tool,daily,start_date,end_date`. Per-user analytics reads `results,total_count,page,page_size,total_pages`, and rows' `user_id,user_email,user_agent,successful_requests,failed_requests,total_requests,total_tokens,spend`. Agent summaries read `results[].{tag,successful_requests,total_tokens,total_spend}`. Trace detail also reads `agents`; span-error pages read `message,next_cursor,span_id,total_chars`. Lens evidence reads `execution,parts,partial,next_cursor`. These names were checked in the retained component types and generated [daily response declarations](https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/ui/litellm-dashboard/src/lib/http/schema.d.ts#L30594-L30610), [evidence declaration](https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/ui/litellm-dashboard/src/lib/http/schema.d.ts#L31423-L31436), and [trace declarations](https://github.com/BerriAI/litellm/blob/d665dc1a6315847fcad6cc2c1264ceb47c06abce/ui/litellm-dashboard/src/lib/http/schema.d.ts#L46509-L46524).

No new product route is assigned in this batch. B9740 supplies independent final-schema fixtures needed to run the missing HTTP cases; B9741-B9744 own their page checks and small patches, and B147 consumes their artifacts in B144's real-browser harness. The fixture and browser shared attachments are explicit architect follow-ups, not hidden edits to other tasks.
