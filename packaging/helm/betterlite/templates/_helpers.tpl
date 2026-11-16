{{/*
Naming, labels, the shared server environment and the small blocks the
gateway, backend, ui and migrations Job templates all render.
*/}}

{{- define "betterlite.name" -}}
{{- default .Chart.Name .Values.nameOverride | trunc 63 | trimSuffix "-" -}}
{{- end -}}

{{- define "betterlite.fullname" -}}
{{- if .Values.fullnameOverride -}}
{{- .Values.fullnameOverride | trunc 63 | trimSuffix "-" -}}
{{- else -}}
{{- $name := default .Chart.Name .Values.nameOverride -}}
{{- printf "%s-%s" .Release.Name $name | trunc 63 | trimSuffix "-" -}}
{{- end -}}
{{- end -}}

{{- define "betterlite.gateway.fullname" -}}
{{- printf "%s-gateway" (include "betterlite.fullname" .) | trunc 63 | trimSuffix "-" -}}
{{- end -}}

{{- define "betterlite.backend.fullname" -}}
{{- printf "%s-backend" (include "betterlite.fullname" .) | trunc 63 | trimSuffix "-" -}}
{{- end -}}

{{- define "betterlite.ui.fullname" -}}
{{- printf "%s-ui" (include "betterlite.fullname" .) | trunc 63 | trimSuffix "-" -}}
{{- end -}}

{{- define "betterlite.commonLabels" -}}
app.kubernetes.io/name: {{ include "betterlite.name" . }}
app.kubernetes.io/instance: {{ .Release.Name }}
app.kubernetes.io/managed-by: {{ .Release.Service }}
helm.sh/chart: {{ printf "%s-%s" .Chart.Name .Chart.Version | replace "+" "_" }}
{{- end -}}

{{- define "betterlite.gateway.selectorLabels" -}}
app.kubernetes.io/name: {{ include "betterlite.name" . }}
app.kubernetes.io/instance: {{ .Release.Name }}
app.kubernetes.io/component: gateway
{{- end -}}

{{- define "betterlite.backend.selectorLabels" -}}
app.kubernetes.io/name: {{ include "betterlite.name" . }}
app.kubernetes.io/instance: {{ .Release.Name }}
app.kubernetes.io/component: backend
{{- end -}}

{{- define "betterlite.ui.selectorLabels" -}}
app.kubernetes.io/name: {{ include "betterlite.name" . }}
app.kubernetes.io/instance: {{ .Release.Name }}
app.kubernetes.io/component: ui
{{- end -}}

{{- define "betterlite.gateway.serviceAccountName" -}}
{{- if .Values.serviceAccounts.gateway.create -}}
{{ default (include "betterlite.gateway.fullname" .) .Values.serviceAccounts.gateway.name }}
{{- else -}}
{{ default "default" .Values.serviceAccounts.gateway.name }}
{{- end -}}
{{- end -}}

{{- define "betterlite.backend.serviceAccountName" -}}
{{- if .Values.serviceAccounts.backend.create -}}
{{ default (include "betterlite.backend.fullname" .) .Values.serviceAccounts.backend.name }}
{{- else -}}
{{ default "default" .Values.serviceAccounts.backend.name }}
{{- end -}}
{{- end -}}

{{- define "betterlite.ui.serviceAccountName" -}}
{{- if .Values.serviceAccounts.ui.create -}}
{{ default (include "betterlite.ui.fullname" .) .Values.serviceAccounts.ui.name }}
{{- else -}}
{{ default "default" .Values.serviceAccounts.ui.name }}
{{- end -}}
{{- end -}}

{{/*
ServiceAccount name for the migrations Job. The Job is a hook, so it runs before
the chart's ordinary resources: an account this chart creates is not there yet,
and the Job falls back to default unless the operator names an existing account.
*/}}
{{- define "betterlite.migrations.serviceAccountName" -}}
{{- if .Values.migrationJob.serviceAccountName -}}
{{ .Values.migrationJob.serviceAccountName }}
{{- else if .Values.serviceAccounts.backend.create -}}
default
{{- else -}}
{{ include "betterlite.backend.serviceAccountName" . }}
{{- end -}}
{{- end -}}

{{/*
Extra pod labels for a Deployment, rejected when they collide with a selector
label: the template would render the key twice, stop matching the immutable
selector, and the apiserver would refuse the Deployment with a vague message.
Invoke with a dict: (dict "podLabels" .Values.gateway.podLabels "componentName" "gateway").
*/}}
{{- define "betterlite.podLabels" -}}
{{- $componentName := .componentName -}}
{{- range $key, $value := .podLabels }}
{{- if has $key (list "app.kubernetes.io/name" "app.kubernetes.io/instance" "app.kubernetes.io/component") }}
{{- fail (printf "%s.podLabels cannot set %s: it is part of the Deployment's immutable selector" $componentName $key) }}
{{- end }}
{{- end }}
{{- toYaml .podLabels }}
{{- end -}}

{{/*
Master key, database and Redis environment shared by the gateway, backend and
migrations Job. Invoke with (dict "root" $ "component" .Values.gateway).

Sensitive values come only from referenced Secrets. The chart never assembles
DATABASE_URL: a raw password in a URL breaks on reserved characters, so the
proxy builds it from the discrete variables.
*/}}
{{- define "betterlite.serverEnv" -}}
{{- $root := .root -}}
{{- $component := .component -}}
- name: LITELLM_MASTER_KEY
  valueFrom:
    secretKeyRef:
      name: {{ required "masterKey.secretName is required (the chart no longer accepts an inline master key)" $root.Values.masterKey.secretName }}
      key: {{ $root.Values.masterKey.secretKey | default "master-key" }}
{{- if $component.logLevel }}
- name: LITELLM_LOG
  value: {{ $component.logLevel | quote }}
{{- end }}
{{- with $root.Values.database.writer }}
- name: DATABASE_HOST
  value: {{ required "database.writer.host is required" .host | quote }}
- name: DATABASE_PORT
  value: {{ .port | default 5432 | quote }}
- name: DATABASE_USER
  valueFrom:
    secretKeyRef:
      name: {{ required "database.writer.passwordSecret.name is required" .passwordSecret.name }}
      key: {{ .passwordSecret.usernameKey | default "username" }}
- name: DATABASE_NAME
  value: {{ required "database.writer.dbname is required" .dbname | quote }}
{{- if .schema }}
- name: DATABASE_SCHEMA
  value: {{ .schema | quote }}
{{- end }}
{{- if .sslMode }}
- name: DATABASE_SSLMODE
  value: {{ .sslMode | quote }}
{{- end }}
{{- if .sslRootCert }}
- name: DATABASE_SSLROOTCERT
  value: {{ .sslRootCert | quote }}
{{- end }}
{{- if and .useIAMAuth .useAzureEntraAuth }}
{{- fail "database.writer.useIAMAuth and database.writer.useAzureEntraAuth are mutually exclusive: the database password can only come from one token source" }}
{{- end }}
{{- if .useIAMAuth }}
- name: IAM_TOKEN_DB_AUTH
  value: "true"
{{- else if .useAzureEntraAuth }}
- name: AZURE_POSTGRESQL_AUTH
  value: "true"
{{- else }}
- name: DATABASE_PASSWORD
  valueFrom:
    secretKeyRef:
      name: {{ .passwordSecret.name }}
      key: {{ .passwordSecret.passwordKey | default "password" }}
{{- end }}
{{- end }}
{{- with $root.Values.database.reader }}
{{- if .host }}
{{- if and .useIAMAuth (not $root.Values.database.writer.useIAMAuth) }}
{{- fail "database.reader.useIAMAuth requires database.writer.useIAMAuth: true (the proxy gates IAM URL minting on IAM_TOKEN_DB_AUTH, which is only set by the writer)" }}
{{- end }}
{{- if and .useAzureEntraAuth (not $root.Values.database.writer.useAzureEntraAuth) }}
{{- fail "database.reader.useAzureEntraAuth requires database.writer.useAzureEntraAuth: true (the proxy gates Entra URL minting on AZURE_POSTGRESQL_AUTH, which is only set by the writer)" }}
{{- end }}
- name: DATABASE_HOST_READ_REPLICA
  value: {{ .host | quote }}
- name: DATABASE_PORT_READ_REPLICA
  value: {{ .port | default 5432 | quote }}
- name: DATABASE_NAME_READ_REPLICA
  value: {{ required "database.reader.dbname is required when database.reader.host is set" .dbname | quote }}
{{- if .schema }}
- name: DATABASE_SCHEMA_READ_REPLICA
  value: {{ .schema | quote }}
{{- end }}
{{- if or .useIAMAuth .useAzureEntraAuth }}
{{- if .passwordSecret.name }}
- name: DATABASE_USER_READ_REPLICA
  valueFrom:
    secretKeyRef:
      name: {{ .passwordSecret.name }}
      key: {{ .passwordSecret.usernameKey | default "username" }}
{{- end }}
{{- else }}
{{- if not .passwordSecret.name }}
{{- fail "database.reader.passwordSecret.name is required when database.reader.host is set" }}
{{- end }}
- name: DATABASE_USER_READ_REPLICA
  valueFrom:
    secretKeyRef:
      name: {{ .passwordSecret.name }}
      key: {{ .passwordSecret.usernameKey | default "username" }}
- name: DATABASE_PASSWORD_READ_REPLICA
  valueFrom:
    secretKeyRef:
      name: {{ .passwordSecret.name }}
      key: {{ .passwordSecret.passwordKey | default "password" }}
{{- end }}
{{- end }}
{{- end }}
{{/*
The migrations Job owns the schema update, so the pods must not run one on
startup and race the Job and each other. The Job entrypoint ignores the variable.
*/}}
- name: DISABLE_SCHEMA_UPDATE
  value: "true"
{{- if $root.Values.redis.host }}
- name: REDIS_HOST
  value: {{ $root.Values.redis.host | quote }}
- name: REDIS_PORT
  value: {{ $root.Values.redis.port | quote }}
{{- if $root.Values.redis.passwordSecret.name }}
- name: REDIS_PASSWORD
  valueFrom:
    secretKeyRef:
      name: {{ $root.Values.redis.passwordSecret.name }}
      key: {{ $root.Values.redis.passwordSecret.passwordKey | default "password" }}
{{- end }}
{{- if $root.Values.redis.cluster }}
{{/* The proxy builds a cluster client from this JSON seed and discovers the other nodes from CLUSTER SLOTS. */}}
- name: REDIS_CLUSTER_NODES
  value: {{ printf "[{\"host\":%q,\"port\":%v}]" $root.Values.redis.host (int $root.Values.redis.port) | quote }}
{{- end }}
{{- end }}
{{- with $component.extraEnv }}
{{ toYaml . }}
{{- end }}
{{- end -}}

{{/*
In-container PgBouncer environment for the gateway container. Under IAM or
Entra auth the pooler mints and renews the database token itself.
*/}}
{{- define "betterlite.connectionPoolEnv" -}}
{{- with .Values.database.connectionPool -}}
- name: LITELLM_PGBOUNCER_ENABLED
  value: "true"
- name: LITELLM_PGBOUNCER_MAX_DB_CONNECTIONS
  value: {{ required "database.connectionPool.maxDbConnections is required when the pool is enabled" .maxDbConnections | quote }}
- name: LITELLM_PGBOUNCER_MAX_CLIENT_CONN
  value: {{ required "database.connectionPool.maxClientConn is required when the pool is enabled" .maxClientConn | quote }}
{{- end }}
{{- end -}}

{{/*
PodDisruptionBudget shared by the three components. Invoke with a dict:
(dict "root" $ "component" .Values.gateway "componentName" "gateway"
      "fullname" (include "betterlite.gateway.fullname" .)
      "selectorLabels" (include "betterlite.gateway.selectorLabels" .))

minAvailable wins over maxUnavailable. With neither set the budget falls back
to maxUnavailable 1. "Set" means non-nil and not the empty string, so an
explicit 0 is written instead of being replaced by the fallback.
*/}}
{{- define "betterlite.pdb" -}}
{{- $root := .root -}}
{{- $component := .component -}}
{{- $min := $component.pdb.minAvailable -}}
{{- $max := $component.pdb.maxUnavailable -}}
{{- $minSet := not (or (kindIs "invalid" $min) (eq (printf "%v" $min) "")) -}}
{{- $maxSet := not (or (kindIs "invalid" $max) (eq (printf "%v" $max) "")) -}}
{{- if and $component.enabled $component.pdb $component.pdb.enabled }}
apiVersion: policy/v1
kind: PodDisruptionBudget
metadata:
  name: {{ .fullname }}
  labels:
    {{- include "betterlite.commonLabels" $root | nindent 4 }}
    app.kubernetes.io/component: {{ .componentName }}
spec:
  selector:
    matchLabels:
      {{- .selectorLabels | nindent 6 }}
  {{- if $minSet }}
  minAvailable: {{ $min }}
  {{- else if $maxSet }}
  maxUnavailable: {{ $max }}
  {{- else }}
  maxUnavailable: 1
  {{- end }}
{{- end }}
{{- end -}}

{{/*
envFrom block for a component's envConfigMaps / envSecrets name lists.
Invoke with the component dict, e.g. .Values.gateway.
*/}}
{{- define "betterlite.envFrom" -}}
{{- $component := . -}}
{{- if or $component.envConfigMaps $component.envSecrets }}
envFrom:
{{- range $component.envConfigMaps }}
  - configMapRef:
      name: {{ . }}
{{- end }}
{{- range $component.envSecrets }}
  - secretRef:
      name: {{ . }}
{{- end }}
{{- end }}
{{- end -}}

{{/*
ingress-nginx's admission webhook rejects a dot in an Exact or Prefix path and
serves ImplementationSpecific as a plain prefix location, so a dotted path
takes that type there.
*/}}
{{- define "betterlite.ingress.pathType" -}}
{{- if and (eq .controller "nginx") (contains "." .path) -}}
ImplementationSpecific
{{- else -}}
{{- .pathType -}}
{{- end -}}
{{- end -}}

{{- define "betterlite.gateway.prometheusMultiprocDir" -}}/tmp/litellm_prometheus_multiproc{{- end -}}

{{/*
Directory of the collector's unix socket, shared by the gateway and collector
containers through an emptyDir. Empty when the sidecar is off or the address is
a tcp://127.0.0.1:<port> endpoint.
*/}}
{{- define "betterlite.gateway.collectorSocketDir" -}}
{{- if and .Values.gateway.collector.enabled (hasPrefix "unix://" .Values.gateway.collector.address) -}}
{{- dir (trimPrefix "unix://" .Values.gateway.collector.address) -}}
{{- end -}}
{{- end -}}

{{/*
LITELLM_COLLECTOR_* environment shared by the producer and the consumer, so
both agree on the transport and the shutdown drain window.
*/}}
{{- define "betterlite.gateway.collectorEnv" -}}
{{- with .Values.gateway.collector }}
- name: LITELLM_COLLECTOR_ENABLED
  value: "true"
- name: LITELLM_COLLECTOR_ADDRESS
  value: {{ .address | quote }}
- name: LITELLM_COLLECTOR_BUFFER_SIZE
  value: {{ .bufferSize | quote }}
- name: LITELLM_COLLECTOR_ON_UNAVAILABLE
  value: {{ .onUnavailable | quote }}
- name: LITELLM_COLLECTOR_DRAIN_TIMEOUT_SECONDS
  value: {{ .drainTimeoutSeconds | quote }}
{{- end }}
{{- end -}}
