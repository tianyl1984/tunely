use crate::{model::*, ssh};
use serde::Serialize;
use std::{
    collections::HashMap,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use tokio::{
    net::TcpListener,
    sync::{Mutex, oneshot, watch},
    task::{JoinHandle, JoinSet},
    time::{interval, timeout},
};

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TunnelStatus {
    pub connection_id: String,
    pub rule_id: String,
    pub state: String,
    pub started_at: Option<u64>,
    pub active_streams: usize,
    pub last_error: Option<String>,
}

struct Entry {
    generation: String,
    status: TunnelStatus,
    cancel: watch::Sender<bool>,
    task: Option<JoinHandle<()>>,
    finished: watch::Receiver<bool>,
}

#[derive(Default)]
pub struct TunnelManager {
    entries: Mutex<HashMap<String, Entry>>,
    shutting_down: AtomicBool,
}

impl TunnelManager {
    pub async fn statuses(&self) -> Vec<TunnelStatus> {
        self.entries
            .lock()
            .await
            .values()
            .map(|entry| entry.status.clone())
            .collect()
    }

    pub async fn active_for(&self, connection_id: &str) -> bool {
        self.entries.lock().await.values().any(|entry| {
            entry.status.connection_id == connection_id
                && ["starting", "running", "stopping"].contains(&entry.status.state.as_str())
        })
    }

    async fn update(&self, id: &str, generation: &str, update: impl FnOnce(&mut TunnelStatus)) {
        if let Some(entry) = self.entries.lock().await.get_mut(id) {
            if entry.generation == generation {
                update(&mut entry.status);
            }
        }
    }

    // Reservation happens before releasing the command lifecycle lock, so edits
    // cannot race with a pending SSH handshake.
    pub async fn launch(
        self: &Arc<Self>,
        connection: Connection,
        rule: ForwardRule,
        credentials: Credentials,
        expected: Option<String>,
    ) -> AppResult<oneshot::Receiver<AppResult<()>>> {
        let mut entries = self.entries.lock().await;
        if self.shutting_down.load(Ordering::Acquire) {
            return Err(AppError::new("应用正在退出，无法开启新通道"));
        }
        if entries.get(&rule.id).is_some_and(|entry| {
            ["starting", "running", "stopping"].contains(&entry.status.state.as_str())
        }) {
            return Err(AppError::new("这条转发正在运行或切换状态"));
        }
        let generation = uuid::Uuid::new_v4().to_string();
        let (cancel, receiver) = watch::channel(false);
        let (ready, response) = oneshot::channel();
        let (finished_sender, finished) = watch::channel(false);
        let manager = self.clone();
        let id = rule.id.clone();
        let task_id = id.clone();
        let task_generation = generation.clone();
        let status = TunnelStatus {
            connection_id: connection.id.clone(),
            rule_id: id.clone(),
            state: "starting".into(),
            started_at: None,
            active_streams: 0,
            last_error: None,
        };
        let task = tokio::spawn(async move {
            let result = manager
                .serve(
                    connection,
                    rule,
                    credentials,
                    expected,
                    receiver,
                    ready,
                    &task_generation,
                )
                .await;
            manager
                .update(&task_id, &task_generation, |status| {
                    status.state = if result.is_err() { "error" } else { "stopped" }.into();
                    status.active_streams = 0;
                    if let Err(error) = result {
                        status.last_error = Some(error.message);
                    }
                })
                .await;
            let _ = finished_sender.send(true);
        });
        entries.insert(
            id,
            Entry {
                generation,
                status,
                cancel,
                task: Some(task),
                finished,
            },
        );
        Ok(response)
    }

    async fn serve(
        &self,
        connection: Connection,
        rule: ForwardRule,
        credentials: Credentials,
        expected: Option<String>,
        mut cancel: watch::Receiver<bool>,
        ready: oneshot::Sender<AppResult<()>>,
        generation: &str,
    ) -> AppResult<()> {
        let setup = async {
            let listener = TcpListener::bind((rule.bind_host.as_str(), rule.local_port))
                .await
                .map_err(|e| {
                    AppError::new(format!(
                        "无法监听 {}:{}，端口可能已被占用：{e}",
                        rule.bind_host, rule.local_port
                    ))
                })?;
            let session = ssh::connect(&connection, credentials, expected).await?;
            Ok::<_, AppError>((listener, session))
        };
        let (listener, session) = tokio::select! {
            biased;
            _ = cancel.changed() => { let _ = ready.send(Err(AppError::new("启动已取消"))); return Ok(()); }
            result = setup => match result {
                Ok(value) => value,
                Err(error) => { let _ = ready.send(Err(error.clone())); return Err(error); }
            }
        };
        self.update(&rule.id, generation, |status| {
            status.state = "running".into();
            status.started_at = Some(
                SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs(),
            );
        })
        .await;
        let _ = ready.send(Ok(()));
        let mut streams = JoinSet::new();
        let mut health = interval(Duration::from_secs(1));
        let result = loop {
            tokio::select! {
                biased;
                _ = cancel.changed() => break Ok(()),
                _ = health.tick() => {
                    if session.handle.is_closed() { break Err(AppError::new("SSH 连接已断开，请检查网络后重新开启")); }
                }
                Some(result) = streams.join_next(), if !streams.is_empty() => {
                    self.update(&rule.id, generation, |status| {
                        status.active_streams = status.active_streams.saturating_sub(1);
                        if let Ok(Err(error)) = result { status.last_error = Some(error); }
                    }).await;
                }
                incoming = listener.accept() => {
                    let (mut socket, peer) = match incoming {
                        Ok(value) => value,
                        Err(error) => break Err(AppError::new(format!("监听失败：{error}"))),
                    };
                    let handle = session.handle.clone();
                    let host = rule.remote_host.clone();
                    let port = rule.remote_port;
                    self.update(&rule.id, generation, |status| { status.active_streams += 1; }).await;
                    streams.spawn(async move {
                        let channel = timeout(ssh::CONNECT_TIMEOUT, handle.channel_open_direct_tcpip(host, port.into(), peer.ip().to_string(), peer.port().into())).await
                            .map_err(|_| "目标端口连接超时".to_owned())?
                            .map_err(|e| format!("无法连接远程目标端口：{e}"))?;
                        let mut remote = channel.into_stream();
                        tokio::io::copy_bidirectional(&mut socket, &mut remote).await
                            .map_err(|e| format!("转发数据连接已结束：{e}"))?;
                        Ok::<_, String>(())
                    });
                }
            }
        };
        drop(listener);
        streams.abort_all();
        while streams.join_next().await.is_some() {}
        session.close().await;
        result
    }

    pub async fn stop(&self, rule_id: &str) -> AppResult<()> {
        let (task, mut finished) = {
            let mut entries = self.entries.lock().await;
            let Some(entry) = entries.get_mut(rule_id) else {
                return Ok(());
            };
            if !["starting", "running", "stopping"].contains(&entry.status.state.as_str()) {
                return Ok(());
            }
            entry.status.state = "stopping".into();
            let _ = entry.cancel.send(true);
            (entry.task.take(), entry.finished.clone())
        };
        if let Some(task) = task {
            let _ = task.await;
        } else if !*finished.borrow() {
            let _ = finished.changed().await;
        }
        Ok(())
    }

    pub async fn stop_all(&self, connection_id: Option<&str>) -> AppResult<()> {
        let ids: Vec<_> = self
            .statuses()
            .await
            .into_iter()
            .filter(|status| {
                connection_id.is_none_or(|id| id == status.connection_id)
                    && ["starting", "running", "stopping"].contains(&status.state.as_str())
            })
            .map(|status| status.rule_id)
            .collect();
        for id in ids {
            self.stop(&id).await?;
        }
        Ok(())
    }

    pub async fn shutdown(&self) -> AppResult<()> {
        // launch checks this flag while holding entries, so stop_all's snapshot
        // includes every reservation made before shutdown and no later launch.
        self.shutting_down.store(true, Ordering::Release);
        self.stop_all(None).await
    }

    pub async fn forget(&self, connection_id: &str) {
        self.entries
            .lock()
            .await
            .retain(|_, entry| entry.status.connection_id != connection_id);
    }
}
