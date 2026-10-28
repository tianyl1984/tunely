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
    time::{interval, sleep, timeout},
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
    pub next_retry_at: Option<u64>,
}

struct Entry {
    generation: String,
    status: TunnelStatus,
    cancel: watch::Sender<bool>,
    retry: watch::Sender<u64>,
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
                && ["starting", "running", "reconnecting", "stopping"]
                    .contains(&entry.status.state.as_str())
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
            ["starting", "running", "reconnecting", "stopping"]
                .contains(&entry.status.state.as_str())
        }) {
            return Err(AppError::new("这条转发正在运行或切换状态"));
        }
        let generation = uuid::Uuid::new_v4().to_string();
        let (cancel, receiver) = watch::channel(false);
        let (retry, retry_receiver) = watch::channel(0);
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
            next_retry_at: None,
        };
        let task = tokio::spawn(async move {
            let result = manager
                .serve(
                    connection,
                    rule,
                    credentials,
                    expected,
                    receiver,
                    retry_receiver,
                    ready,
                    &task_generation,
                )
                .await;
            manager
                .update(&task_id, &task_generation, |status| {
                    status.state = if result.is_err() { "error" } else { "stopped" }.into();
                    status.active_streams = 0;
                    status.next_retry_at = None;
                    status.last_error = result.err().map(|error| error.message);
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
                retry,
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
        mut retry: watch::Receiver<u64>,
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
            let session = ssh::connect(&connection, credentials.clone(), expected.clone()).await?;
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
        let mut session = session;
        let mut ready = Some(ready);
        loop {
            self.update(&rule.id, generation, |status| {
                status.state = "running".into();
                status.started_at = Some(now_seconds());
                status.last_error = None;
                status.next_retry_at = None;
            })
            .await;
            if let Some(ready) = ready.take() {
                let _ = ready.send(Ok(()));
            }
            let result = self
                .forward_session(&listener, &session, &rule, &mut cancel, generation)
                .await;
            session.close().await;
            let Err(mut error) = result else {
                return Ok(());
            };
            let mut backoff = ReconnectBackoff::default();
            'retry: loop {
                // Host key changes always require explicit verification via a new start.
                if error.host_key.is_some() {
                    return Err(error);
                }
                let (delay, attempt) = backoff.next_wait();
                self.update(&rule.id, generation, |status| {
                    status.state = "reconnecting".into();
                    status.active_streams = 0;
                    status.last_error = Some(error.message.clone());
                    status.next_retry_at = Some(now_seconds() + delay.as_secs());
                })
                .await;
                tokio::select! {
                    biased;
                    _ = cancel.changed() => return Ok(()),
                    _ = retry.changed() => { backoff = ReconnectBackoff::default(); }
                    _ = sleep(delay) => {
                        if !attempt {
                            return Err(AppError::new(format!("累计等待已达 5 分钟，已停止自动重连：{}", error.message)));
                        }
                    }
                }
                loop {
                    self.update(&rule.id, generation, |status| {
                        status.next_retry_at = None;
                    })
                    .await;
                    let result = tokio::select! {
                        biased;
                        _ = cancel.changed() => return Ok(()),
                        _ = retry.changed() => { backoff = ReconnectBackoff::default(); continue; }
                        result = ssh::connect(&connection, credentials.clone(), expected.clone()) => result,
                    };
                    match result {
                        Ok(connected) => {
                            session = connected;
                            break 'retry;
                        }
                        Err(cause) => {
                            error = cause;
                            continue 'retry;
                        }
                    }
                }
            }
        }
    }

    async fn forward_session(
        &self,
        listener: &TcpListener,
        session: &ssh::SshSession,
        rule: &ForwardRule,
        cancel: &mut watch::Receiver<bool>,
        generation: &str,
    ) -> AppResult<()> {
        let mut streams = JoinSet::new();
        let mut health = interval(Duration::from_secs(1));
        let result = loop {
            tokio::select! {
                biased;
                _ = cancel.changed() => break Ok(()),
                _ = health.tick() => {
                    if session.handle.is_closed() { break Err(AppError::new("SSH 连接已断开")); }
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
        streams.abort_all();
        while streams.join_next().await.is_some() {}
        result
    }

    pub async fn reconnect(&self, rule_id: &str) -> AppResult<()> {
        let entries = self.entries.lock().await;
        let entry = entries
            .get(rule_id)
            .ok_or_else(|| AppError::new("转发规则未运行"))?;
        if entry.status.state != "reconnecting" {
            return Err(AppError::new("当前转发不在自动重连期间"));
        }
        entry
            .retry
            .send_modify(|value| *value = value.wrapping_add(1));
        Ok(())
    }

    pub async fn stop(&self, rule_id: &str) -> AppResult<()> {
        let (task, mut finished) = {
            let mut entries = self.entries.lock().await;
            let Some(entry) = entries.get_mut(rule_id) else {
                return Ok(());
            };
            if !["starting", "running", "reconnecting", "stopping"]
                .contains(&entry.status.state.as_str())
            {
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
                    && ["starting", "running", "reconnecting", "stopping"]
                        .contains(&status.state.as_str())
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

fn now_seconds() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

#[derive(Default)]
struct ReconnectBackoff {
    waited: u64,
    attempt: u32,
}

impl ReconnectBackoff {
    fn next_wait(&mut self) -> (Duration, bool) {
        let delay = 1u64 << self.attempt.min(9);
        let remaining = 300 - self.waited;
        let attempt = delay < remaining;
        let wait = delay.min(remaining);
        self.waited += wait;
        self.attempt += 1;
        (Duration::from_secs(wait), attempt)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reconnect_backoff_stops_at_five_minutes_and_can_reset() {
        let mut backoff = ReconnectBackoff::default();
        for seconds in [1, 2, 4, 8, 16, 32, 64, 128] {
            assert_eq!(backoff.next_wait(), (Duration::from_secs(seconds), true));
        }
        assert_eq!(backoff.next_wait(), (Duration::from_secs(45), false));
        assert_eq!(backoff.waited, 300);
        backoff = ReconnectBackoff::default();
        assert_eq!(backoff.next_wait(), (Duration::from_secs(1), true));
    }
}
