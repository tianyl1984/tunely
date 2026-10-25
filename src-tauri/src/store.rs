use crate::model::*;
use serde::{Deserialize, Serialize};
use std::{collections::HashMap, fs, io::Write, path::PathBuf};

#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Config {
    #[serde(default)]
    pub connections: Vec<Connection>,
    #[serde(default)]
    pub trusted_hosts: HashMap<String, String>,
}

pub struct Store {
    pub path: PathBuf,
    pub config: Config,
}

impl Store {
    pub fn open(directory: PathBuf) -> AppResult<Self> {
        fs::create_dir_all(&directory)
            .map_err(|e| AppError::new(format!("无法创建配置目录：{e}")))?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&directory, fs::Permissions::from_mode(0o700))
                .map_err(|e| AppError::new(format!("无法设置配置目录权限：{e}")))?;
        }
        let path = directory.join("connections.json");
        let config = match fs::read(&path) {
            Ok(bytes) => serde_json::from_slice(&bytes)
                .map_err(|e| AppError::new(format!("本地配置格式错误，原文件已保留：{e}")))?,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Config::default(),
            Err(e) => return Err(AppError::new(format!("无法读取配置：{e}"))),
        };
        Ok(Self { path, config })
    }

    pub fn commit(&mut self, config: Config) -> AppResult<()> {
        let directory = self
            .path
            .parent()
            .ok_or_else(|| AppError::new("无效的配置目录"))?;
        let mut file = tempfile::NamedTempFile::new_in(directory)
            .map_err(|e| AppError::new(format!("无法创建临时配置：{e}")))?;
        let bytes = serde_json::to_vec_pretty(&config).map_err(|e| AppError::new(e.to_string()))?;
        file.write_all(&bytes)
            .and_then(|_| file.as_file().sync_all())
            .map_err(|e| AppError::new(format!("无法写入配置：{e}")))?;
        file.persist(&self.path)
            .map_err(|e| AppError::new(format!("无法保存配置：{e}")))?;
        self.config = config;
        Ok(())
    }

    pub fn connection(&self, id: &str) -> AppResult<Connection> {
        self.config
            .connections
            .iter()
            .find(|c| c.id == id)
            .cloned()
            .ok_or_else(|| AppError::new("连接配置不存在"))
    }

    pub fn expected_key(&self, connection: &Connection) -> Option<String> {
        self.config
            .trusted_hosts
            .get(&host_key(&connection.host, connection.port))
            .cloned()
    }
}

pub fn host_key(host: &str, port: u16) -> String {
    format!("[{}]:{}", host.to_lowercase(), port)
}

fn credential_entry(id: &str) -> AppResult<keyring::Entry> {
    keyring::Entry::new("com.tunely.desktop", id)
        .map_err(|e| AppError::new(format!("无法打开系统凭据库：{e}")))
}

pub fn read_credentials(id: &str) -> AppResult<Credentials> {
    match credential_entry(id)?.get_password() {
        Ok(value) => serde_json::from_str(&value)
            .map_err(|_| AppError::new("本地凭据已损坏，请重新输入密码")),
        Err(keyring::Error::NoEntry) => Ok(Credentials::default()),
        Err(e) => Err(AppError::new(format!("无法读取系统凭据库：{e}"))),
    }
}

pub fn write_credentials(id: &str, credentials: &Credentials) -> AppResult<()> {
    if credentials.password.is_none() && credentials.passphrase.is_none() {
        return delete_credentials(id);
    }
    credential_entry(id)?
        .set_password(
            &serde_json::to_string(credentials).map_err(|e| AppError::new(e.to_string()))?,
        )
        .map_err(|e| AppError::new(format!("无法保存到系统凭据库：{e}")))
}

pub fn delete_credentials(id: &str) -> AppResult<()> {
    match credential_entry(id)?.delete_credential() {
        Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
        Err(e) => Err(AppError::new(format!("无法删除系统凭据：{e}"))),
    }
}

pub fn resolve_credentials(
    input: &ConnectionInput,
    previous: Option<&Connection>,
    mut credentials: Credentials,
) -> AppResult<Credentials> {
    if input.connection.auth_type == AuthType::Password {
        if let Some(password) = &input.password {
            credentials.password = Some(password.clone());
        }
        credentials.passphrase = None;
        if credentials.password.as_deref().unwrap_or("").is_empty() {
            return Err(AppError::new("请输入 SSH 登录密码"));
        }
    } else {
        credentials.password = None;
        if input.clear_passphrase
            || previous.is_some_and(|p| p.private_key_path != input.connection.private_key_path)
        {
            credentials.passphrase = None;
        }
        if let Some(passphrase) = &input.passphrase {
            credentials.passphrase = Some(passphrase.clone());
        }
    }
    Ok(credentials)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn atomic_config_round_trip_and_corruption_protection() {
        let directory = tempfile::tempdir().unwrap();
        let mut store = Store::open(directory.path().into()).unwrap();
        let mut config = Config::default();
        config
            .trusted_hosts
            .insert("[example.com]:22".into(), "SHA256:test".into());
        store.commit(config).unwrap();
        let reopened = Store::open(directory.path().into()).unwrap();
        assert_eq!(
            reopened.config.trusted_hosts["[example.com]:22"],
            "SHA256:test"
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                fs::metadata(&store.path).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
        fs::write(&store.path, b"invalid JSON").unwrap();
        assert!(Store::open(directory.path().into()).is_err());
        assert_eq!(fs::read(&store.path).unwrap(), b"invalid JSON");
    }
}
