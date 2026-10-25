use crate::{
    model::*,
    ssh,
    store::{Config, Store, resolve_credentials},
    tunnel::TunnelManager,
};
use russh::{
    Channel, ChannelOpenFailure,
    keys::{Algorithm, HashAlg, PrivateKey, PublicKey, ssh_key::LineEnding},
    server::{self, Server as _},
};
use std::{net::SocketAddr, sync::Arc, time::Duration};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    task::JoinHandle,
    time::{sleep, timeout},
};

#[derive(Clone)]
struct TestServer {
    accepted_key: PublicKey,
}
impl server::Server for TestServer {
    type Handler = Self;
    fn new_client(&mut self, _: Option<SocketAddr>) -> Self {
        self.clone()
    }
}
impl server::Handler for TestServer {
    type Error = russh::Error;
    async fn auth_password(
        &mut self,
        user: &str,
        password: &str,
    ) -> Result<server::Auth, Self::Error> {
        Ok(if user == "tester" && password == "test-password" {
            server::Auth::Accept
        } else {
            server::Auth::reject()
        })
    }
    async fn auth_publickey(
        &mut self,
        user: &str,
        key: &PublicKey,
    ) -> Result<server::Auth, Self::Error> {
        Ok(if user == "tester" && key == &self.accepted_key {
            server::Auth::Accept
        } else {
            server::Auth::reject()
        })
    }
    async fn channel_open_direct_tcpip(
        &mut self,
        channel: Channel<server::Msg>,
        host: &str,
        port: u32,
        _: &str,
        _: u32,
        reply: server::ChannelOpenHandle,
        _: &mut server::Session,
    ) -> Result<(), Self::Error> {
        match TcpStream::connect((host, port as u16)).await {
            Ok(mut target) => {
                reply.accept().await;
                tokio::spawn(async move {
                    let mut stream = channel.into_stream();
                    let _ = tokio::io::copy_bidirectional(&mut stream, &mut target).await;
                });
            }
            Err(_) => {
                reply.reject(ChannelOpenFailure::ConnectFailed).await;
            }
        }
        Ok(())
    }
}

struct Fixture {
    connection: Connection,
    key: PrivateKey,
    fingerprint: String,
    shutdown: server::RunningServerHandle,
    task: JoinHandle<()>,
}
impl Fixture {
    async fn new() -> Self {
        let server_key = PrivateKey::random(&mut rand::rng(), Algorithm::Ed25519).unwrap();
        let fingerprint = server_key
            .public_key()
            .fingerprint(HashAlg::Sha256)
            .to_string();
        let key = PrivateKey::random(&mut rand::rng(), Algorithm::Ed25519).unwrap();
        let mut server = TestServer {
            accepted_key: key.public_key().clone(),
        };
        let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let config = Arc::new(server::Config {
            keys: vec![server_key],
            auth_rejection_time: Duration::ZERO,
            auth_rejection_time_initial: Some(Duration::ZERO),
            ..Default::default()
        });
        let (ready, receiver) = tokio::sync::oneshot::channel();
        let task = tokio::spawn(async move {
            let running = server.run_on_socket(config, &listener);
            ready.send(running.handle()).ok();
            running.await.unwrap();
        });
        let shutdown = receiver.await.unwrap();
        Self {
            connection: Connection {
                id: uuid::Uuid::new_v4().to_string(),
                name: "Local test SSH".into(),
                host: "127.0.0.1".into(),
                port,
                username: "tester".into(),
                auth_type: AuthType::Password,
                private_key_path: String::new(),
                has_password: true,
                has_passphrase: false,
                rules: vec![],
            },
            key,
            fingerprint,
            shutdown,
            task,
        }
    }
    fn credentials(&self) -> Credentials {
        Credentials {
            password: Some("test-password".into()),
            passphrase: None,
        }
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        self.shutdown.shutdown("Test complete".into());
        self.task.abort();
    }
}

async fn free_port() -> u16 {
    let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
    listener.local_addr().unwrap().port()
}
fn rule(local: u16, remote: u16, bind: &str) -> ForwardRule {
    ForwardRule {
        id: uuid::Uuid::new_v4().to_string(),
        name: "Test forward".into(),
        bind_host: bind.into(),
        local_port: local,
        remote_host: "127.0.0.1".into(),
        remote_port: remote,
    }
}
async fn echo_server() -> (u16, JoinHandle<()>) {
    let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let task = tokio::spawn(async move {
        while let Ok((socket, _)) = listener.accept().await {
            tokio::spawn(async move {
                let (mut reader, mut writer) = socket.into_split();
                let _ = tokio::io::copy(&mut reader, &mut writer).await;
            });
        }
    });
    (port, task)
}

#[tokio::test]
async fn host_key_confirmation_change_detection_and_password_auth() {
    let fixture = Fixture::new().await;
    let unknown = ssh::connect(&fixture.connection, fixture.credentials(), None)
        .await
        .err()
        .unwrap();
    assert_eq!(unknown.host_key.unwrap().fingerprint, fixture.fingerprint);
    let changed = ssh::connect(
        &fixture.connection,
        fixture.credentials(),
        Some("SHA256:previous-key".into()),
    )
    .await
    .err()
    .unwrap();
    assert_eq!(
        changed.host_key.unwrap().previous_fingerprint.as_deref(),
        Some("SHA256:previous-key")
    );
    let invalid = ssh::connect(
        &fixture.connection,
        Credentials {
            password: Some("wrong-password".into()),
            passphrase: None,
        },
        Some(fixture.fingerprint.clone()),
    )
    .await;
    assert!(invalid.err().unwrap().message.contains("认证失败"));
    let session = ssh::connect(
        &fixture.connection,
        fixture.credentials(),
        Some(fixture.fingerprint.clone()),
    )
    .await
    .unwrap();
    assert_eq!(session.fingerprint, fixture.fingerprint);
    session.close().await;
}

#[tokio::test]
async fn encrypted_private_key_authentication() {
    let fixture = Fixture::new().await;
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("id_ed25519");
    let encrypted = fixture
        .key
        .encrypt(&mut rand::rng(), "test-passphrase")
        .unwrap();
    std::fs::write(
        &path,
        encrypted.to_openssh(LineEnding::LF).unwrap().as_bytes(),
    )
    .unwrap();
    let mut connection = fixture.connection.clone();
    connection.auth_type = AuthType::PrivateKey;
    connection.private_key_path = path.to_string_lossy().into();
    let bad = ssh::connect(
        &connection,
        Credentials {
            password: None,
            passphrase: Some("wrong".into()),
        },
        Some(fixture.fingerprint.clone()),
    )
    .await;
    assert!(bad.err().unwrap().message.contains("私钥"));
    let session = ssh::connect(
        &connection,
        Credentials {
            password: None,
            passphrase: Some("test-passphrase".into()),
        },
        Some(fixture.fingerprint.clone()),
    )
    .await
    .unwrap();
    session.close().await;
}

#[tokio::test]
async fn forwards_concurrent_clients_and_releases_both_bind_addresses() {
    let fixture = Fixture::new().await;
    let (remote, echo) = echo_server().await;
    let manager = Arc::new(TunnelManager::default());
    for bind in ["127.0.0.1", "0.0.0.0"] {
        let rule = rule(free_port().await, remote, bind);
        manager
            .launch(
                fixture.connection.clone(),
                rule.clone(),
                fixture.credentials(),
                Some(fixture.fingerprint.clone()),
            )
            .await
            .unwrap()
            .await
            .unwrap()
            .unwrap();
        let mut clients = tokio::task::JoinSet::new();
        for index in 0..4 {
            let port = rule.local_port;
            clients.spawn(async move {
                let mut client = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
                let payload = format!("client-{index}:{}", "0123456789".repeat(10000));
                client.write_all(payload.as_bytes()).await.unwrap();
                client.shutdown().await.unwrap();
                let mut received = vec![];
                timeout(Duration::from_secs(5), client.read_to_end(&mut received))
                    .await
                    .unwrap()
                    .unwrap();
                assert_eq!(received, payload.as_bytes());
            });
        }
        while let Some(result) = clients.join_next().await {
            result.unwrap();
        }
        let mut open_client = TcpStream::connect(("127.0.0.1", rule.local_port))
            .await
            .unwrap();
        open_client.write_all(b"alive").await.unwrap();
        let mut buffer = [0u8; 5];
        timeout(Duration::from_secs(3), open_client.read_exact(&mut buffer))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(&buffer, b"alive");
        manager.stop(&rule.id).await.unwrap();
        assert_eq!(
            manager
                .statuses()
                .await
                .iter()
                .find(|s| s.rule_id == rule.id)
                .unwrap()
                .state,
            "stopped"
        );
        let result = timeout(Duration::from_secs(2), open_client.read(&mut buffer))
            .await
            .unwrap();
        assert!(matches!(result, Ok(0) | Err(_)));
        let rebound = TcpListener::bind((bind, rule.local_port)).await.unwrap();
        drop(rebound);
    }
    echo.abort();
}

#[tokio::test]
async fn occupied_port_cancellation_and_disconnect_are_reported() {
    let fixture = Fixture::new().await;
    let occupied = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
    let manager = Arc::new(TunnelManager::default());
    let conflicting = rule(occupied.local_addr().unwrap().port(), 8080, "127.0.0.1");
    let error = manager
        .launch(
            fixture.connection.clone(),
            conflicting,
            fixture.credentials(),
            Some(fixture.fingerprint.clone()),
        )
        .await
        .unwrap()
        .await
        .unwrap()
        .unwrap_err();
    assert!(error.message.contains("无法监听"));
    let hanging = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
    let mut connection = fixture.connection.clone();
    connection.port = hanging.local_addr().unwrap().port();
    let cancelled = rule(free_port().await, 8080, "127.0.0.1");
    let response = manager
        .launch(connection, cancelled.clone(), fixture.credentials(), None)
        .await
        .unwrap();
    manager.stop(&cancelled.id).await.unwrap();
    assert!(response.await.unwrap().is_err());
    assert!(
        TcpListener::bind(("127.0.0.1", cancelled.local_port))
            .await
            .is_ok()
    );
    let active = rule(free_port().await, 8080, "127.0.0.1");
    manager
        .launch(
            fixture.connection.clone(),
            active.clone(),
            fixture.credentials(),
            Some(fixture.fingerprint.clone()),
        )
        .await
        .unwrap()
        .await
        .unwrap()
        .unwrap();
    fixture.shutdown.shutdown("Simulated network loss".into());
    timeout(Duration::from_secs(5), async {
        loop {
            if manager
                .statuses()
                .await
                .iter()
                .any(|s| s.rule_id == active.id && s.state == "error")
            {
                break;
            }
            sleep(Duration::from_millis(50)).await;
        }
    })
    .await
    .unwrap();
    assert!(
        TcpListener::bind(("127.0.0.1", active.local_port))
            .await
            .is_ok()
    );
}

#[tokio::test]
async fn config_persists_rules_and_never_serializes_credentials() {
    let fixture = Fixture::new().await;
    let mut connection = fixture.connection.clone();
    connection.rules = vec![rule(8000, 8080, "127.0.0.1"), rule(9000, 9090, "0.0.0.0")];
    connection.validate(true).unwrap();
    let directory = tempfile::tempdir().unwrap();
    let mut store = Store::open(directory.path().into()).unwrap();
    store
        .commit(Config {
            connections: vec![connection.clone()],
            ..Default::default()
        })
        .unwrap();
    let reopened = Store::open(directory.path().into()).unwrap();
    let loaded = reopened.connection(&connection.id).unwrap();
    assert_eq!(loaded.rules[1].bind_host, "0.0.0.0");
    assert_eq!(loaded.rules[0].remote_port, 8080);
    let json = std::fs::read_to_string(&store.path).unwrap();
    assert!(!json.contains("test-password"));
    assert!(!json.contains("\"password\":"));
    let input = ConnectionInput {
        connection: connection.clone(),
        password: None,
        passphrase: None,
        clear_passphrase: false,
    };
    let retained = resolve_credentials(&input, Some(&connection), fixture.credentials()).unwrap();
    assert_eq!(retained.password.as_deref(), Some("test-password"));
    connection.rules[1].local_port = 8000;
    assert!(
        connection
            .validate(true)
            .unwrap_err()
            .message
            .contains("重复")
    );
}

#[tokio::test]
async fn stopping_one_connection_preserves_others_and_stop_is_idempotent() {
    let fixture = Fixture::new().await;
    let manager = Arc::new(TunnelManager::default());
    let first = rule(free_port().await, 8080, "127.0.0.1");
    let second = rule(free_port().await, 8080, "127.0.0.1");
    let mut other_connection = fixture.connection.clone();
    other_connection.id = uuid::Uuid::new_v4().to_string();
    for (connection, rule) in [
        (fixture.connection.clone(), first.clone()),
        (other_connection, second.clone()),
    ] {
        manager
            .launch(
                connection,
                rule,
                fixture.credentials(),
                Some(fixture.fingerprint.clone()),
            )
            .await
            .unwrap()
            .await
            .unwrap()
            .unwrap();
    }
    let (a, b) = tokio::join!(
        manager.stop(&first.id),
        manager.stop_all(Some(&fixture.connection.id))
    );
    a.unwrap();
    b.unwrap();
    let statuses = manager.statuses().await;
    assert_eq!(
        statuses
            .iter()
            .find(|s| s.rule_id == first.id)
            .unwrap()
            .state,
        "stopped"
    );
    assert_eq!(
        statuses
            .iter()
            .find(|s| s.rule_id == second.id)
            .unwrap()
            .state,
        "running"
    );
    manager.stop_all(None).await.unwrap();
    manager.stop(&second.id).await.unwrap();
    assert!(
        TcpListener::bind(("127.0.0.1", second.local_port))
            .await
            .is_ok()
    );
}

#[test]
#[ignore = "Requires access to the local OS credential store"]
fn system_credentials_round_trip() {
    let id = format!("test-{}", uuid::Uuid::new_v4());
    let credentials = Credentials {
        password: Some("temporary-fixture-password".into()),
        passphrase: None,
    };
    crate::store::write_credentials(&id, &credentials).unwrap();
    let loaded = crate::store::read_credentials(&id);
    let deleted = crate::store::delete_credentials(&id);
    assert_eq!(loaded.unwrap().password, credentials.password);
    deleted.unwrap();
    assert!(
        crate::store::read_credentials(&id)
            .unwrap()
            .password
            .is_none()
    );
}

#[tokio::test]
async fn shutdown_drains_running_and_starting_tunnels_and_rejects_new_launches() {
    let fixture = Fixture::new().await;
    let (remote, echo) = echo_server().await;
    let manager = Arc::new(TunnelManager::default());
    let active = rule(free_port().await, remote, "127.0.0.1");
    manager
        .launch(
            fixture.connection.clone(),
            active.clone(),
            fixture.credentials(),
            Some(fixture.fingerprint.clone()),
        )
        .await
        .unwrap()
        .await
        .unwrap()
        .unwrap();
    let mut client = TcpStream::connect(("127.0.0.1", active.local_port))
        .await
        .unwrap();
    client.write_all(b"alive").await.unwrap();
    let mut buffer = [0; 5];
    client.read_exact(&mut buffer).await.unwrap();
    assert_eq!(&buffer, b"alive");

    // Keep a second connection in the SSH handshake while exit is requested.
    let hanging = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
    let mut connection = fixture.connection.clone();
    connection.id = uuid::Uuid::new_v4().to_string();
    connection.port = hanging.local_addr().unwrap().port();
    let pending = rule(free_port().await, remote, "0.0.0.0");
    let response = manager
        .launch(connection, pending.clone(), fixture.credentials(), None)
        .await
        .unwrap();

    timeout(Duration::from_secs(5), async {
        let (first, second) = tokio::join!(manager.shutdown(), manager.shutdown());
        first.unwrap();
        second.unwrap();
    })
    .await
    .expect("exit must not wait for the pending SSH handshake to time out");
    assert!(response.await.unwrap().is_err());
    assert!(
        manager
            .statuses()
            .await
            .iter()
            .all(|status| { status.state == "stopped" && status.active_streams == 0 })
    );
    let result = timeout(Duration::from_secs(2), client.read(&mut buffer))
        .await
        .unwrap();
    assert!(matches!(result, Ok(0) | Err(_)));
    for rule in [&active, &pending] {
        let listener = TcpListener::bind((rule.bind_host.as_str(), rule.local_port))
            .await
            .expect("exit must release every listening port");
        drop(listener);
    }
    let rejected = manager
        .launch(
            fixture.connection.clone(),
            active,
            fixture.credentials(),
            Some(fixture.fingerprint.clone()),
        )
        .await
        .unwrap_err();
    assert!(rejected.message.contains("应用正在退出"));
    echo.abort();
}
