//! GitHub release discovery, verified staging and a separate restart helper.
//! Credentials stay in memory; downloaded code is never run during a check.
mod install;

use anyhow::{Context, Result, bail};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, Sender};
use std::time::Duration;

pub const REPO: &str = "fikrifirmanf/ferrum-ssh";
pub const RELEASES_URL: &str = "https://github.com/fikrifirmanf/ferrum-ssh/releases";
const MAX_ASSET: u64 = 256 * 1024 * 1024;

#[derive(Clone, Debug, Deserialize)]
pub struct Asset {
    pub id: u64,
    pub name: String,
    pub size: u64,
    pub digest: Option<String>,
}
#[derive(Clone, Debug, Deserialize)]
pub struct Release {
    pub tag_name: String,
    #[serde(default)]
    pub body: Option<String>,
    pub draft: bool,
    pub prerelease: bool,
    pub assets: Vec<Asset>,
}

pub struct Prepared {
    pub version: String,
    pub directory: tempfile::TempDir,
    pub payload: PathBuf,
}

pub enum State {
    Idle,
    Checking,
    Current(String),
    Available(Release),
    Downloading(u64, u64),
    Ready(Prepared),
    Failed(String),
}

enum Event {
    Checked {
        result: Result<Option<Release>, String>,
        notify: bool,
    },
    Progress(u64, u64),
    Prepared(Result<Prepared, String>),
}

pub struct Updater {
    pub state: State,
    pub token: String,
    pub previews: bool,
    notice_pending: bool,
    dismissed_tag: Option<String>,
    tx: Sender<Event>,
    rx: Receiver<Event>,
}
impl Default for Updater {
    fn default() -> Self {
        let (tx, rx) = mpsc::channel();
        Self {
            state: State::Idle,
            token: String::new(),
            previews: false,
            notice_pending: false,
            dismissed_tag: None,
            tx,
            rx,
        }
    }
}
impl Updater {
    pub fn busy(&self) -> bool {
        matches!(self.state, State::Checking | State::Downloading(..))
    }
    pub fn poll(&mut self) {
        while let Ok(event) = self.rx.try_recv() {
            self.state = match event {
                Event::Checked { result, notify } => {
                    self.notice_pending = false;
                    match result {
                        Ok(Some(release)) => {
                            self.notice_pending =
                                notify && self.dismissed_tag.as_deref() != Some(&release.tag_name);
                            State::Available(release)
                        }
                        Ok(None) => State::Current(format!(
                            "Ferrum {} is up to date for this release channel.",
                            env!("CARGO_PKG_VERSION")
                        )),
                        Err(error) => State::Failed(error),
                    }
                }
                Event::Prepared(Err(error)) => State::Failed(error),
                Event::Progress(done, total) => State::Downloading(done, total),
                Event::Prepared(Ok(prepared)) => State::Ready(prepared),
            };
        }
    }
    pub fn notice(&self) -> Option<&Release> {
        if !self.notice_pending {
            return None;
        }
        match &self.state {
            State::Available(release) => Some(release),
            _ => None,
        }
    }

    pub fn dismiss_notice(&mut self) {
        if let State::Available(release) = &self.state {
            self.dismissed_tag = Some(release.tag_name.clone());
        }
        self.notice_pending = false;
    }

    pub fn check_automatically(&mut self, rt: &tokio::runtime::Handle, ctx: &egui::Context) {
        self.start_check(rt, ctx, true);
    }

    pub fn check(&mut self, rt: &tokio::runtime::Handle, ctx: &egui::Context) {
        self.start_check(rt, ctx, false);
    }

    fn start_check(&mut self, rt: &tokio::runtime::Handle, ctx: &egui::Context, notify: bool) {
        if self.busy() {
            return;
        }
        self.state = State::Checking;
        let (tx, ctx, token, previews) = (
            self.tx.clone(),
            ctx.clone(),
            self.token.clone(),
            self.previews,
        );
        rt.spawn(async move {
            let result = async {
                let client = client()?;
                // Startup checks use the public release API without accessing credentials.
                let token = if notify { String::new() } else { resolve_token(token).await };
                let response = authorized(client.get(format!("https://api.github.com/repos/{REPO}/releases?per_page=100")), &token)
                    .send().await.context("Could not reach GitHub. Check your connection.")?;
                if response.status() == reqwest::StatusCode::NOT_FOUND {
                    bail!("GitHub could not find this repository. For private releases, sign in with GitHub CLI or enter a token with read access.");
                }
                if response.status() == reqwest::StatusCode::UNAUTHORIZED || response.status() == reqwest::StatusCode::FORBIDDEN {
                    bail!("GitHub denied access. Check your token permissions or API rate limit.");
                }
                let releases: Vec<Release> = response.error_for_status()?.json().await?;
                select_release(releases, env!("CARGO_PKG_VERSION"), previews)
            }.await.map_err(|e: anyhow::Error| format!("{e:#}"));
            let _ = tx.send(Event::Checked { result, notify });
            ctx.request_repaint();
        });
    }
    pub fn download(&mut self, rt: &tokio::runtime::Handle, ctx: &egui::Context) {
        let State::Available(release) = &self.state else {
            return;
        };
        let (release, token, tx, ctx) = (
            release.clone(),
            self.token.clone(),
            self.tx.clone(),
            ctx.clone(),
        );
        self.state = State::Downloading(0, 0);
        rt.spawn(async move {
            let result = stage_release(release, token, &tx, &ctx)
                .await
                .map_err(|e| format!("{e:#}"));
            let _ = tx.send(Event::Prepared(result));
            ctx.request_repaint();
        });
    }
    pub fn install(&mut self) -> Result<()> {
        let State::Ready(prepared) = &self.state else {
            bail!("Download and verify an update first.")
        };
        install::launch(prepared)?;
        // The helper owns cleanup after this process exits.
        let State::Ready(prepared) = std::mem::replace(&mut self.state, State::Idle) else {
            unreachable!()
        };
        let _ = prepared.directory.keep();
        Ok(())
    }
}

fn client() -> Result<reqwest::Client> {
    Ok(reqwest::Client::builder()
        .user_agent(concat!("Ferrum/", env!("CARGO_PKG_VERSION")))
        .https_only(true)
        .connect_timeout(Duration::from_secs(15))
        .timeout(Duration::from_secs(300))
        .build()?)
}
fn authorized(request: reqwest::RequestBuilder, token: &str) -> reqwest::RequestBuilder {
    let request = request.header("X-GitHub-Api-Version", "2022-11-28");
    if token.is_empty() {
        request
    } else {
        request.bearer_auth(token)
    }
}
async fn resolve_token(explicit: String) -> String {
    if !explicit.trim().is_empty() {
        return explicit.trim().into();
    }
    for name in ["GH_TOKEN", "GITHUB_TOKEN"] {
        if let Ok(token) = std::env::var(name) {
            if !token.trim().is_empty() {
                return token;
            }
        }
    }
    // Reuse a user's existing gh login without saving a second copy of its token.
    for executable in ["gh", "/opt/homebrew/bin/gh", "/usr/local/bin/gh"] {
        if let Ok(Ok(output)) = tokio::time::timeout(
            Duration::from_secs(5),
            tokio::process::Command::new(executable)
                .args(["auth", "token", "--hostname", "github.com"])
                .kill_on_drop(true)
                .output(),
        )
        .await
        {
            if output.status.success() {
                return String::from_utf8_lossy(&output.stdout).trim().into();
            }
        }
    }
    String::new()
}

fn version(tag: &str) -> Result<semver::Version> {
    Ok(semver::Version::parse(
        tag.strip_prefix('v').unwrap_or(tag),
    )?)
}
fn select_release(
    releases: Vec<Release>,
    current: &str,
    previews: bool,
) -> Result<Option<Release>> {
    let current = version(current)?;
    Ok(releases
        .into_iter()
        .filter(|r| !r.draft && (previews || !r.prerelease))
        .filter_map(|r| version(&r.tag_name).ok().map(|v| (v, r)))
        .filter(|(v, _)| *v > current && (previews || v.pre.is_empty()))
        .max_by(|(a, _), (b, _)| a.cmp(b))
        .map(|(_, r)| r))
}
fn asset_name(tag: &str, os: &str, arch: &str) -> Result<String> {
    let version = version(tag)?;
    let suffix = match (os, arch) {
        ("macos", "aarch64") => "macos-arm64.dmg",
        ("macos", "x86_64") => "macos-x86_64.dmg",
        ("windows", "x86_64") => "windows-x86_64.zip",
        ("linux", "x86_64") => "linux-x86_64.tar.gz",
        _ => bail!("No update package is available for {os}/{arch}."),
    };
    Ok(format!("Ferrum-{version}-{suffix}"))
}
fn expected_digest(asset: &Asset) -> Result<String> {
    let digest = asset.digest.as_deref().and_then(|d| d.strip_prefix("sha256:"))
        .filter(|d| d.len() == 64 && d.bytes().all(|b| b.is_ascii_hexdigit()))
        .context("This release asset has no valid SHA-256 digest. Download it from the release page instead.")?;
    Ok(digest.to_ascii_lowercase())
}
fn verify_digest(actual: &[u8], expected: &str) -> Result<()> {
    if hex::encode(actual) != expected {
        bail!("The update checksum did not match. The download was discarded. Try again.");
    }
    Ok(())
}
async fn stage_release(
    release: Release,
    token: String,
    tx: &Sender<Event>,
    ctx: &egui::Context,
) -> Result<Prepared> {
    let name = asset_name(
        &release.tag_name,
        std::env::consts::OS,
        std::env::consts::ARCH,
    )?;
    let asset = release
        .assets
        .iter()
        .find(|a| a.name == name)
        .context("This release does not include a package for your computer yet.")?;
    let digest = expected_digest(asset)?;
    if asset.size == 0 || asset.size > MAX_ASSET {
        bail!("The update package size is invalid.");
    }
    let client = client()?;
    let token = resolve_token(token).await;
    let mut response = authorized(
        client.get(format!(
            "https://api.github.com/repos/{REPO}/releases/assets/{}",
            asset.id
        )),
        &token,
    )
    .header("Accept", "application/octet-stream")
    .send()
    .await?
    .error_for_status()?;
    let dir = tempfile::Builder::new()
        .prefix("ferrum-update-")
        .tempdir()?;
    let archive = dir.path().join(&name);
    use tokio::io::AsyncWriteExt;
    let mut file = tokio::fs::File::create(&archive).await?;
    let mut hash = Sha256::new();
    let mut downloaded = 0;
    while let Some(chunk) = response.chunk().await? {
        downloaded += chunk.len() as u64;
        if downloaded > asset.size || downloaded > MAX_ASSET {
            bail!("The update is larger than GitHub reported.");
        }
        hash.update(&chunk);
        file.write_all(&chunk).await?;
        let _ = tx.send(Event::Progress(downloaded, asset.size));
        ctx.request_repaint();
    }
    file.sync_all().await?;
    drop(file);
    if downloaded != asset.size {
        bail!("The update download was incomplete. Try again.");
    }
    verify_digest(&hash.finalize(), &digest)?;
    let tag = release.tag_name.clone();
    tokio::task::spawn_blocking(move || {
        let payload = install::extract(&archive, dir.path(), &tag)?;
        Ok(Prepared {
            version: tag,
            directory: dir,
            payload,
        })
    })
    .await?
}

#[cfg(test)]
mod tests;

#[cfg(debug_assertions)]
impl Updater {
    pub fn preview_notice(&mut self) {
        let _ = self.tx.send(Event::Checked {
            notify: true,
            result: Ok(Some(Release {
                tag_name: "v0.2.0".into(),
                // unslop-ignore NO_SQL_INTERPOLATION -- Plain release-note text in a debug fixture; no SQL is executed.
                body: Some("Demo update for visual verification.".into()),
                draft: false,
                prerelease: false,
                assets: vec![],
            })),
        });
        self.poll();
    }
}
