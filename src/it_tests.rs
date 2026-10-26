//! End-to-end tests against a real SSH server. Opt-in because they need one:
//!
//! ```sh
//! FERRUM_IT_SSH="user@127.0.0.1:2222" FERRUM_IT_KEY=/path/to/private_key \
//!     cargo test it_ -- --ignored --test-threads=1
//! ```

use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::mpsc::unbounded_channel;

use crate::conn::{AppEvent, Bridge, ConnCtxTemplate, PromptReply};
use crate::model::{Host, PortForward, ResolvedTarget, Vault};
use crate::sftp::{self, Backend};

struct Env {
    target: ResolvedTarget,
    tpl: ConnCtxTemplate,
    events: tokio::sync::mpsc::UnboundedReceiver<AppEvent>,
}

/// Builds a target from the env vars; auto-accepts the host-key prompt like a user would.
fn setup() -> Env {
    let spec = std::env::var("FERRUM_IT_SSH").expect("set FERRUM_IT_SSH=user@host:port");
    let key_path = std::env::var("FERRUM_IT_KEY").expect("set FERRUM_IT_KEY=/path/to/private_key");
    let (user, host, port) = crate::model::parse_quick_connect(&spec).expect("bad FERRUM_IT_SSH");
    let key = crate::keys::import("it", &std::fs::read_to_string(key_path).unwrap(), "").unwrap();
    let mut vault = Vault::default();
    let mut h = Host {
        label: "it".into(),
        address: host,
        ..Default::default()
    };
    h.settings.username = user;
    h.settings.port = port;
    h.settings.key_id = Some(key.id);
    h.settings.use_agent = Some(false);
    vault.keys.push(key);
    let target = vault.resolve(&h);

    let (tx, mut raw) = unbounded_channel();
    let (fwd_tx, events) = unbounded_channel();
    tokio::spawn(async move {
        while let Some(ev) = raw.recv().await {
            match ev {
                AppEvent::Prompt(req) => {
                    let _ = req.reply.send(PromptReply::Accept { save: false });
                }
                other => {
                    let _ = fwd_tx.send(other);
                }
            }
        }
    });
    let tpl = ConnCtxTemplate {
        bridge: Bridge {
            events: tx,
            ctx: egui::Context::default(),
        },
        known_hosts: vec![],
        mosh_client: String::new(),
        detect_os: false,
        local_shell: String::new(),
    };
    Env {
        target,
        tpl,
        events,
    }
}

#[tokio::test]
#[ignore = "needs a live SSH server; see module docs"]
async fn it_exec_runs_remote_command() {
    let env = setup();
    let client = crate::conn::ssh::connect_headless(env.target.clone(), env.tpl.headless())
        .await
        .unwrap();
    let (out, code) = client.exec("echo ferrum-$((6*7))").await.unwrap();
    assert_eq!(out.trim(), "ferrum-42");
    assert_eq!(code, 0);
    let (_, code) = client.exec("exit 3").await.unwrap();
    assert_eq!(code, 3);
}

#[tokio::test]
#[ignore = "needs a live SSH server; see module docs"]
async fn it_sftp_round_trip() {
    let env = setup();
    let client = crate::conn::ssh::connect_headless(env.target.clone(), env.tpl.headless())
        .await
        .unwrap();
    let remote = Backend::Remote {
        sftp: std::sync::Arc::new(client.sftp().await.unwrap()),
        client: client.clone(),
        label: "it".into(),
    };

    let dir = format!("/tmp/ferrum-it-{}", uuid::Uuid::new_v4());
    sftp::mkdir(&remote, &dir).await.unwrap();
    let local_src =
        std::env::temp_dir().join(format!("ferrum-it-src-{}.txt", uuid::Uuid::new_v4()));
    let payload = "hello over sftp\n".repeat(5000); // ~80 KB, several chunks
    std::fs::write(&local_src, &payload).unwrap();

    let remote_file = format!("{dir}/upload.txt");
    sftp::upload_file(&remote, &local_src, &remote_file)
        .await
        .unwrap();

    let listing = sftp::list(&remote, &dir).await.unwrap();
    let entry = listing
        .iter()
        .find(|e| e.name == "upload.txt")
        .expect("uploaded file listed");
    assert_eq!(entry.size, payload.len() as u64);

    sftp::rename(&remote, &remote_file, &format!("{dir}/renamed.txt"))
        .await
        .unwrap();
    sftp::chmod(&remote, &format!("{dir}/renamed.txt"), 0o600)
        .await
        .unwrap();
    let listing = sftp::list(&remote, &dir).await.unwrap();
    let renamed = listing
        .iter()
        .find(|e| e.name == "renamed.txt")
        .expect("renamed file listed");
    assert_eq!(renamed.perms.map(|p| p & 0o777), Some(0o600));

    let local_copy = sftp::download_for_edit(&remote, &format!("{dir}/renamed.txt"))
        .await
        .unwrap();
    assert_eq!(std::fs::read_to_string(&local_copy).unwrap(), payload);

    sftp::remove(&remote, &dir, true).await.unwrap();
    let (out, _) = client
        .exec(&format!("test -e {dir} && echo still-there || echo gone"))
        .await
        .unwrap();
    assert_eq!(out.trim(), "gone");
    std::fs::remove_file(local_src).ok();
}

#[tokio::test]
#[ignore = "needs a live SSH server; see module docs"]
async fn it_local_port_forward_reaches_destination() {
    let mut env = setup();
    // Tunnel a local port to the SSH server's own port and expect its banner back.
    let rule = PortForward {
        bind_address: "127.0.0.1".into(),
        bind_port: 18_022,
        dest_host: "127.0.0.1".into(),
        dest_port: env.target.port,
        ..Default::default()
    };
    let mut mgr = crate::forward::ForwardManager::default();
    mgr.start(
        &tokio::runtime::Handle::current(),
        rule.clone(),
        env.target.clone(),
        env.tpl.headless(),
    );

    let running = tokio::time::timeout(Duration::from_secs(10), async {
        while let Some(ev) = env.events.recv().await {
            if let AppEvent::ForwardState { running, error, .. } = ev {
                return (running, error);
            }
        }
        (false, Some("event channel closed".into()))
    })
    .await
    .expect("forward never reported its state");
    assert!(running.0, "forward failed: {:?}", running.1);

    let mut sock = tokio::net::TcpStream::connect(("127.0.0.1", 18_022))
        .await
        .unwrap();
    let mut banner = [0u8; 8];
    tokio::time::timeout(Duration::from_secs(5), sock.read_exact(&mut banner))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(&banner, b"SSH-2.0-");
    sock.shutdown().await.ok();
    mgr.stop(rule.id, &env.tpl.bridge);
}

#[tokio::test]
#[ignore = "needs a live SSH server; see module docs"]
async fn it_strict_host_key_check_refuses_unknown_host() {
    let env = setup();
    let mut strict = env.target.clone();
    strict.strict_host_key_check = true;
    let err = crate::conn::ssh::connect_headless(strict, env.tpl.headless())
        .await
        .err()
        .expect("unknown host must be refused in strict mode");
    assert!(
        format!("{err:#}").contains("strict host key checking"),
        "got: {err:#}"
    );
    // The same host connects fine when not strict (the harness auto-accepts the prompt).
    crate::conn::ssh::connect_headless(env.target.clone(), env.tpl.headless())
        .await
        .unwrap();
}

#[tokio::test]
#[ignore = "needs a live SSH server; see module docs"]
async fn it_connect_timeout_is_honoured() {
    let env = setup();
    let mut t = env.target.clone();
    // 192.0.2.0/24 is reserved for documentation: packets are dropped, never answered.
    t.address = "192.0.2.1".into();
    t.connect_timeout_secs = 2;
    t.use_agent = false;
    let started = std::time::Instant::now();
    let err = crate::conn::ssh::connect_headless(t, env.tpl.headless())
        .await
        .err()
        .expect("must fail");
    assert!(
        started.elapsed() < Duration::from_secs(6),
        "took {:?}",
        started.elapsed()
    );
    let msg = format!("{err:#}");
    assert!(
        msg.contains("timed out") || msg.contains("unreachable") || msg.contains("cannot reach"),
        "got: {msg}"
    );
}
