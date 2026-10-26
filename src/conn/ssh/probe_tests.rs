//! Hermetic SSH probe tests against an in-process loopback server.
use super::*;
use crate::conn::{ConnCtxTemplate, PromptReply};
use crate::model::{Host, Vault};
use russh::server::{self, Server as _};

#[derive(Clone)]
struct TestServer {
    commands: Arc<Mutex<Vec<Vec<u8>>>>,
    shells: Arc<Mutex<usize>>,
}

impl server::Server for TestServer {
    type Handler = Self;
    fn new_client(&mut self, _: Option<std::net::SocketAddr>) -> Self {
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
        Ok(if user == "probe" && password == "correct" {
            server::Auth::Accept
        } else {
            server::Auth::reject()
        })
    }
    async fn channel_open_session(
        &mut self,
        _: russh::Channel<server::Msg>,
        reply: server::ChannelOpenHandle,
        _: &mut server::Session,
    ) -> Result<(), Self::Error> {
        reply.accept().await;
        Ok(())
    }
    async fn exec_request(
        &mut self,
        channel: russh::ChannelId,
        data: &[u8],
        session: &mut server::Session,
    ) -> Result<(), Self::Error> {
        self.commands.lock().push(data.to_vec());
        session.channel_success(channel)?;
        session.data(channel, b"PRETTY_NAME=\"Ubuntu 24.04 LTS\"\n".to_vec())?;
        session.exit_status_request(channel, 0)?;
        session.eof(channel)?;
        session.close(channel)?;
        Ok(())
    }
    async fn shell_request(
        &mut self,
        _: russh::ChannelId,
        _: &mut server::Session,
    ) -> Result<(), Self::Error> {
        *self.shells.lock() += 1;
        Ok(())
    }
}

struct Fixture {
    target: ResolvedTarget,
    ctx: ConnCtx,
    server: TestServer,
    tasks: Vec<tokio::task::JoinHandle<()>>,
}
impl Drop for Fixture {
    fn drop(&mut self) {
        for task in &self.tasks {
            task.abort();
        }
    }
}

async fn fixture(password: &str) -> Fixture {
    let key = ssh_key::PrivateKey::from(ssh_key::private::Ed25519Keypair::from_seed(&[42; 32]));
    let socket = tokio::net::TcpListener::bind(("127.0.0.1", 0))
        .await
        .unwrap();
    let port = socket.local_addr().unwrap().port();
    let known = KnownHost {
        id: uuid::Uuid::nil(),
        host: "127.0.0.1".into(),
        port,
        key_type: "ssh-ed25519".into(),
        public_key: key.public_key().to_openssh().unwrap(),
        fingerprint: fingerprint(key.public_key()),
        added: 0,
        updated: 0,
    };
    let server = TestServer {
        commands: Default::default(),
        shells: Default::default(),
    };
    let mut running = server.clone();
    let config = Arc::new(server::Config {
        keys: vec![key],
        auth_rejection_time: Duration::ZERO,
        auth_rejection_time_initial: Some(Duration::ZERO),
        ..Default::default()
    });
    let task = tokio::spawn(async move {
        running.run_on_socket(config, &socket).await.unwrap();
    });
    let (tx, mut events) = tokio::sync::mpsc::unbounded_channel();
    let prompts = tokio::spawn(async move {
        while let Some(ev) = events.recv().await {
            if let AppEvent::Prompt(req) = ev {
                let _ = req.reply.send(PromptReply::Cancel);
            }
        }
    });
    let ctx = ConnCtxTemplate {
        bridge: Bridge {
            events: tx,
            ctx: egui::Context::default(),
        },
        known_hosts: vec![known],
        mosh_client: String::new(),
        detect_os: false,
        local_shell: String::new(),
    }
    .headless();
    let mut host = Host {
        address: "127.0.0.1".into(),
        ..Default::default()
    };
    host.settings.port = Some(port);
    host.settings.username = Some("probe".into());
    host.settings.password = Some(password.into());
    host.settings.use_agent = Some(false);
    Fixture {
        target: Vault::default().resolve(&host),
        ctx,
        server,
        tasks: vec![task, prompts],
    }
}

#[tokio::test]
async fn probe_authenticates_detects_os_without_opening_shell() {
    let f = fixture("correct").await;
    let result = test_connection(f.target.clone(), f.ctx.clone()).await;
    assert_eq!(result, Ok(Some("Ubuntu 24.04 LTS".into())));
    assert_eq!(*f.server.shells.lock(), 0);
    assert_eq!(
        *f.server.commands.lock(),
        vec![b"cat /etc/os-release 2>/dev/null || uname -sr".to_vec()]
    );
}

#[tokio::test]
async fn probe_rejects_bad_credentials() {
    let f = fixture("wrong").await;
    let error = test_connection(f.target.clone(), f.ctx.clone())
        .await
        .unwrap_err();
    assert!(error.contains("cancelled"), "{error}");
    assert!(f.server.commands.lock().is_empty());
}

#[tokio::test]
async fn probe_refuses_changed_key_with_strict_checking() {
    let mut f = fixture("correct").await;
    f.ctx.known_hosts[0].fingerprint = "SHA256:different".into();
    f.target.strict_host_key_check = true;
    let error = test_connection(f.target.clone(), f.ctx.clone())
        .await
        .unwrap_err();
    assert!(error.contains("different from the saved one"), "{error}");
    assert!(f.server.commands.lock().is_empty());
}
