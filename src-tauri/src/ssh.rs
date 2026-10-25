use crate::model::*;
use russh::{
    Disconnect, client,
    keys::{HashAlg, PrivateKeyWithHashAlg, PublicKeyOrCertificate},
};
use std::{
    path::PathBuf,
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::time::timeout;

pub const CONNECT_TIMEOUT: Duration = Duration::from_secs(15);

pub struct HostVerifier {
    host: String,
    port: u16,
    expected: Option<String>,
    observed: Arc<Mutex<Option<HostKeyChallenge>>>,
}

impl client::Handler for HostVerifier {
    type Error = russh::Error;

    async fn check_server_key(
        &mut self,
        key: &PublicKeyOrCertificate,
    ) -> Result<bool, Self::Error> {
        let fingerprint = key.public_key().fingerprint(HashAlg::Sha256).to_string();
        let accepted = self.expected.as_ref() == Some(&fingerprint);
        *self.observed.lock().unwrap() = Some(HostKeyChallenge {
            host: self.host.clone(),
            port: self.port,
            fingerprint,
            previous_fingerprint: self.expected.clone(),
        });
        Ok(accepted)
    }
}

// A cancelled/failed authentication must also close its SSH transport.
struct ConnectingGuard(Option<client::Handle<HostVerifier>>);
impl Drop for ConnectingGuard {
    fn drop(&mut self) {
        if let Some(handle) = self.0.take() {
            tokio::spawn(async move {
                let _ = timeout(
                    Duration::from_secs(2),
                    handle.disconnect(Disconnect::ByApplication, "Tunely closing", ""),
                )
                .await;
            });
        }
    }
}

pub struct SshSession {
    pub handle: Arc<client::Handle<HostVerifier>>,
    pub fingerprint: String,
}

impl SshSession {
    pub async fn close(&self) {
        let _ = timeout(
            Duration::from_secs(2),
            self.handle
                .disconnect(Disconnect::ByApplication, "Tunely stopped", ""),
        )
        .await;
    }
}

impl Drop for SshSession {
    fn drop(&mut self) {
        let handle = self.handle.clone();
        tokio::spawn(async move {
            let _ = timeout(
                Duration::from_secs(2),
                handle.disconnect(Disconnect::ByApplication, "Tunely closing", ""),
            )
            .await;
        });
    }
}

fn expand_key_path(value: &str) -> PathBuf {
    if let Some(rest) = value.strip_prefix("~/") {
        if let Some(home) = std::env::var_os("HOME") {
            return PathBuf::from(home).join(rest);
        }
    }
    PathBuf::from(value)
}

pub async fn connect(
    connection: &Connection,
    credentials: Credentials,
    expected: Option<String>,
) -> AppResult<SshSession> {
    let observed = Arc::new(Mutex::new(None));
    let verifier = HostVerifier {
        host: connection.host.clone(),
        port: connection.port,
        expected,
        observed: observed.clone(),
    };
    let operation = async {
        let config = Arc::new(client::Config {
            keepalive_interval: Some(Duration::from_secs(15)),
            keepalive_max: 3,
            inactivity_timeout: None,
            nodelay: true,
            ..Default::default()
        });
        let handle = client::connect(
            config,
            (connection.host.as_str(), connection.port),
            verifier,
        )
        .await
        .map_err(|e| {
            let observation = observed.lock().unwrap().clone();
            if let Some(key) = observation {
                if key.previous_fingerprint.as_ref() != Some(&key.fingerprint) {
                    return AppError {
                        message: if key.previous_fingerprint.is_some() {
                            "服务器指纹已变化，请核实后再连接"
                        } else {
                            "首次连接，请确认服务器指纹"
                        }
                        .into(),
                        host_key: Some(key),
                    };
                }
            }
            AppError::new(format!("无法建立 SSH 连接：{e}"))
        })?;
        let mut guard = ConnectingGuard(Some(handle));
        let handle = guard.0.as_mut().unwrap();
        let result = match connection.auth_type {
            AuthType::Password => {
                handle
                    .authenticate_password(
                        &connection.username,
                        credentials.password.unwrap_or_default(),
                    )
                    .await
            }
            AuthType::PrivateKey => {
                let path = expand_key_path(&connection.private_key_path);
                let passphrase = credentials.passphrase;
                let key = tokio::task::spawn_blocking(move || {
                    russh::keys::load_secret_key(path, passphrase.as_deref())
                })
                .await
                .map_err(|e| AppError::new(format!("读取私钥失败：{e}")))?
                .map_err(|e| AppError::new(format!("无法读取私钥，请检查文件及口令：{e}")))?;
                let hash = handle
                    .best_supported_rsa_hash()
                    .await
                    .map_err(|e| AppError::new(format!("协商密钥算法失败：{e}")))?
                    .flatten();
                handle
                    .authenticate_publickey(
                        &connection.username,
                        PrivateKeyWithHashAlg::new(Arc::new(key), hash),
                    )
                    .await
            }
        }
        .map_err(|e| AppError::new(format!("SSH 认证失败：{e}")))?;
        if !result.success() {
            return Err(AppError::new(
                "SSH 认证失败，请检查用户名、密码或私钥是否正确",
            ));
        }
        let fingerprint = observed
            .lock()
            .unwrap()
            .as_ref()
            .map(|key| key.fingerprint.clone())
            .unwrap_or_default();
        Ok(SshSession {
            handle: Arc::new(guard.0.take().unwrap()),
            fingerprint,
        })
    };
    timeout(CONNECT_TIMEOUT, operation)
        .await
        .map_err(|_| AppError::new("SSH 连接超时（15 秒），请检查主机、端口及网络"))?
}
