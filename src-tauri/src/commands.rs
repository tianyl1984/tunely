use crate::{
    model::*,
    ssh,
    store::{self, Store},
    tunnel::{TunnelManager, TunnelStatus},
};
use serde::Serialize;
use std::sync::{Arc, Mutex};
use tauri::{AppHandle, State};
use tauri_plugin_opener::OpenerExt;

pub struct AppState {
    pub store: Arc<Mutex<Store>>,
    pub tunnels: Arc<TunnelManager>,
    pub lifecycle: tokio::sync::Mutex<()>,
}

async fn storage<T: Send + 'static>(
    state: &AppState,
    action: impl FnOnce(&mut Store) -> AppResult<T> + Send + 'static,
) -> AppResult<T> {
    let store = state.store.clone();
    tokio::task::spawn_blocking(move || {
        let mut store = store
            .lock()
            .map_err(|_| AppError::new("配置存储正忙，请重启应用"))?;
        action(&mut store)
    })
    .await
    .map_err(|e| AppError::new(format!("本地存储操作失败：{e}")))?
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Snapshot {
    connections: Vec<Connection>,
    config_path: String,
}

#[tauri::command]
pub async fn load_connections(state: State<'_, AppState>) -> AppResult<Snapshot> {
    storage(&state, |store| {
        Ok(Snapshot {
            connections: store.config.connections.clone(),
            config_path: store.path.to_string_lossy().into(),
        })
    })
    .await
}

#[tauri::command]
pub async fn open_local_port(
    app: AppHandle,
    state: State<'_, AppState>,
    connection_id: String,
    rule_id: String,
) -> AppResult<()> {
    let port = storage(&state, move |store| {
        let connection = store.connection(&connection_id)?;
        let rule = connection
            .rules
            .iter()
            .find(|rule| rule.id == rule_id)
            .ok_or_else(|| AppError::new("转发规则不存在"))?;
        if rule.local_port == 0 {
            return Err(AppError::new("本地端口无效"));
        }
        Ok(rule.local_port)
    })
    .await?;
    app.opener()
        .open_url(format!("http://127.0.0.1:{port}"), None::<&str>)
        .map_err(|error| AppError::new(format!("无法打开默认浏览器：{error}")))
}

#[tauri::command]
pub async fn save_connection(
    state: State<'_, AppState>,
    mut input: ConnectionInput,
) -> AppResult<Connection> {
    input.connection.normalize();
    input.connection.validate(true)?;
    let _lifecycle = state.lifecycle.lock().await;
    if state.tunnels.active_for(&input.connection.id).await {
        return Err(AppError::new("请先停止此连接下的转发，再修改配置"));
    }
    let id = input.connection.id.clone();
    let connection = storage(&state, move |store| {
        let previous = store
            .config
            .connections
            .iter()
            .find(|c| c.id == input.connection.id);
        if store
            .config
            .connections
            .iter()
            .filter(|c| c.id != input.connection.id)
            .flat_map(|c| &c.rules)
            .any(|r| {
                input
                    .connection
                    .rules
                    .iter()
                    .any(|candidate| candidate.id == r.id)
            })
        {
            return Err(AppError::new("转发规则 ID 与其他连接重复"));
        }
        let old_credentials = if previous.is_some() {
            store::read_credentials(&input.connection.id)?
        } else {
            Credentials::default()
        };
        let credentials = store::resolve_credentials(&input, previous, old_credentials.clone())?;
        input.connection.has_password = credentials.password.is_some();
        input.connection.has_passphrase = credentials.passphrase.is_some();
        let mut config = store.config.clone();
        if let Some(existing) = config
            .connections
            .iter_mut()
            .find(|c| c.id == input.connection.id)
        {
            *existing = input.connection.clone();
        } else {
            config.connections.push(input.connection.clone());
        }
        store::write_credentials(&input.connection.id, &credentials)?;
        if let Err(error) = store.commit(config) {
            let _ = store::write_credentials(&input.connection.id, &old_credentials);
            return Err(error);
        }
        Ok(input.connection)
    })
    .await?;
    state.tunnels.forget(&id).await;
    Ok(connection)
}

#[tauri::command]
pub async fn delete_connection(state: State<'_, AppState>, connection_id: String) -> AppResult<()> {
    let _lifecycle = state.lifecycle.lock().await;
    state.tunnels.stop_all(Some(&connection_id)).await?;
    let id = connection_id.clone();
    storage(&state, move |store| {
        store.connection(&id)?;
        let credentials = store::read_credentials(&id)?;
        let mut config = store.config.clone();
        config.connections.retain(|connection| connection.id != id);
        store::delete_credentials(&id)?;
        if let Err(error) = store.commit(config) {
            let _ = store::write_credentials(&id, &credentials);
            return Err(error);
        }
        Ok(())
    })
    .await?;
    state.tunnels.forget(&connection_id).await;
    Ok(())
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TestResult {
    message: String,
    fingerprint: String,
    latency_ms: u128,
}

#[tauri::command]
pub async fn test_connection(
    state: State<'_, AppState>,
    mut input: ConnectionInput,
) -> AppResult<TestResult> {
    input.connection.normalize();
    input.connection.validate(false)?;
    let (connection, credentials, expected) = storage(&state, move |store| {
        let previous = store
            .config
            .connections
            .iter()
            .find(|c| c.id == input.connection.id);
        let credentials = if previous.is_some() {
            store::read_credentials(&input.connection.id)?
        } else {
            Credentials::default()
        };
        let credentials = store::resolve_credentials(&input, previous, credentials)?;
        let expected = store.expected_key(&input.connection);
        Ok((input.connection, credentials, expected))
    })
    .await?;
    let start = std::time::Instant::now();
    let session = ssh::connect(&connection, credentials, expected).await?;
    let result = TestResult {
        message: "SSH 认证成功，可以开启端口转发".into(),
        fingerprint: session.fingerprint.clone(),
        latency_ms: start.elapsed().as_millis(),
    };
    session.close().await;
    Ok(result)
}

#[tauri::command]
pub async fn trust_host_key(
    state: State<'_, AppState>,
    challenge: HostKeyChallenge,
) -> AppResult<()> {
    if !challenge.fingerprint.starts_with("SHA256:") || challenge.fingerprint.len() < 20 {
        return Err(AppError::new("无效的服务器指纹"));
    }
    storage(&state, move |store| {
        let key = store::host_key(&challenge.host, challenge.port);
        if store.config.trusted_hosts.get(&key) != challenge.previous_fingerprint.as_ref() {
            return Err(AppError::new("服务器信任记录已更新，请重新测试连接"));
        }
        let mut config = store.config.clone();
        config.trusted_hosts.insert(key, challenge.fingerprint);
        store.commit(config)
    })
    .await
}

#[tauri::command]
pub async fn start_forward(
    state: State<'_, AppState>,
    connection_id: String,
    rule_id: String,
) -> AppResult<()> {
    let response = {
        let _lifecycle = state.lifecycle.lock().await;
        let (connection, rule, credentials, expected) = storage(&state, move |store| {
            let connection = store.connection(&connection_id)?;
            connection.validate(true)?;
            let rule = connection
                .rules
                .iter()
                .find(|r| r.id == rule_id)
                .cloned()
                .ok_or_else(|| AppError::new("转发规则不存在"))?;
            let credentials = store::read_credentials(&connection.id)?;
            let expected = store.expected_key(&connection);
            Ok((connection, rule, credentials, expected))
        })
        .await?;
        state
            .tunnels
            .launch(connection, rule, credentials, expected)
            .await?
    };
    response
        .await
        .map_err(|_| AppError::new("转发任务异常退出，请重试"))?
}

#[tauri::command]
pub async fn stop_forward(state: State<'_, AppState>, rule_id: String) -> AppResult<()> {
    state.tunnels.stop(&rule_id).await
}

#[tauri::command]
pub async fn stop_all_forwards(
    state: State<'_, AppState>,
    connection_id: Option<String>,
) -> AppResult<()> {
    state.tunnels.stop_all(connection_id.as_deref()).await
}

#[tauri::command]
pub async fn tunnel_statuses(state: State<'_, AppState>) -> AppResult<Vec<TunnelStatus>> {
    Ok(state.tunnels.statuses().await)
}

#[tauri::command]
pub async fn reconnect_forward(state: State<'_, AppState>, rule_id: String) -> AppResult<()> {
    state.tunnels.reconnect(&rule_id).await
}
