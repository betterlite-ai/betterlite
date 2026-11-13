#[derive(Debug, Clone, Default)]
pub struct Connection {
    pub db: Option<i64>,
    pub username: Option<String>,
    pub ssl: Option<bool>,
    pub socket_connect_timeout: Option<f64>,
    pub socket_keepalive: Option<bool>,
    pub max_connections: Option<i64>,
    pub health_check_interval: Option<i64>,
}

impl Connection {
    pub(super) fn or(self, other: Self) -> Self {
        Self {
            db: self.db.or(other.db),
            username: self.username.or(other.username),
            ssl: self.ssl.or(other.ssl),
            socket_connect_timeout: self.socket_connect_timeout.or(other.socket_connect_timeout),
            socket_keepalive: self.socket_keepalive.or(other.socket_keepalive),
            max_connections: self.max_connections.or(other.max_connections),
            health_check_interval: self.health_check_interval.or(other.health_check_interval),
        }
    }
}
