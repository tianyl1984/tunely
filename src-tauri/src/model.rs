use serde::{Deserialize, Serialize};
use std::collections::HashSet;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub enum AuthType {
    Password,
    PrivateKey,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ForwardRule {
    pub id: String,
    pub name: String,
    pub bind_host: String,
    pub local_port: u16,
    pub remote_host: String,
    pub remote_port: u16,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Connection {
    pub id: String,
    pub name: String,
    pub host: String,
    pub port: u16,
    pub username: String,
    pub auth_type: AuthType,
    #[serde(default)]
    pub private_key_path: String,
    #[serde(default)]
    pub has_password: bool,
    #[serde(default)]
    pub has_passphrase: bool,
    #[serde(default)]
    pub rules: Vec<ForwardRule>,
}

// Credentials are accepted only as command input, never returned to the webview.
#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConnectionInput {
    pub connection: Connection,
    pub password: Option<String>,
    pub passphrase: Option<String>,
    #[serde(default)]
    pub clear_passphrase: bool,
}

#[derive(Clone, Default, Serialize, Deserialize)]
pub struct Credentials {
    pub password: Option<String>,
    pub passphrase: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HostKeyChallenge {
    pub host: String,
    pub port: u16,
    pub fingerprint: String,
    pub previous_fingerprint: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppError {
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub host_key: Option<HostKeyChallenge>,
}

impl AppError {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            host_key: None,
        }
    }
}

pub type AppResult<T> = Result<T, AppError>;

fn valid_id(value: &str) -> bool {
    uuid::Uuid::parse_str(value).is_ok()
}

fn valid_host(value: &str) -> bool {
    !value.is_empty() && value.len() <= 253 && !value.chars().any(char::is_whitespace)
}

impl Connection {
    pub fn normalize(&mut self) {
        self.name = self.name.trim().to_owned();
        self.host = self.host.trim().to_owned();
        self.username = self.username.trim().to_owned();
        self.private_key_path = self.private_key_path.trim().to_owned();
        for rule in &mut self.rules {
            rule.name = rule.name.trim().to_owned();
            rule.remote_host = rule.remote_host.trim().to_owned();
            if rule.bind_host == "localhost" {
                rule.bind_host = "127.0.0.1".into();
            }
        }
    }

    pub fn validate(&self, include_rules: bool) -> AppResult<()> {
        if !valid_id(&self.id) {
            return Err(AppError::new("连接 ID 无效"));
        }
        if self.name.is_empty() || self.name.len() > 160 {
            return Err(AppError::new("请填写连接名称（最多 160 字节）"));
        }
        if !valid_host(&self.host) || self.port == 0 || self.username.is_empty() {
            return Err(AppError::new("请填写有效的 SSH 主机、端口和用户名"));
        }
        if self.auth_type == AuthType::PrivateKey && self.private_key_path.is_empty() {
            return Err(AppError::new("请选择私钥文件"));
        }
        if !include_rules {
            return Ok(());
        }
        let mut ids = HashSet::new();
        let mut ports = HashSet::new();
        for rule in &self.rules {
            if !valid_id(&rule.id) || !ids.insert(&rule.id) {
                return Err(AppError::new("转发规则 ID 无效或重复"));
            }
            if !["127.0.0.1", "0.0.0.0"].contains(&rule.bind_host.as_str()) {
                return Err(AppError::new("监听地址必须为 localhost 或 0.0.0.0"));
            }
            if rule.local_port == 0 || rule.remote_port == 0 || !valid_host(&rule.remote_host) {
                return Err(AppError::new("请填写有效的转发端口（1–65535）和目标主机"));
            }
            if !ports.insert(rule.local_port) {
                return Err(AppError::new("同一连接中的本地端口不能重复"));
            }
        }
        Ok(())
    }
}
