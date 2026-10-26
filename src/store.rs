//! Encrypted local vault storage and folder-based end-to-end encrypted sync.
//!
//! On-disk format: `FERRUM1` magic, 1 byte mode (0 = device key, 1 = master password),
//! 16 byte salt, 24 byte nonce, XChaCha20-Poly1305 ciphertext of the JSON vault.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow, bail};
use chacha20poly1305::XChaCha20Poly1305;
use chacha20poly1305::aead::{Aead, KeyInit};
use rand::RngCore;

use crate::model::{Vault, now_ts};

const MAGIC: &[u8; 7] = b"FERRUM1";

pub fn data_dir() -> PathBuf {
    let dirs = directories::ProjectDirs::from("dev", "Ferrum", "Ferrum");
    let dir = dirs
        .map(|d| d.data_dir().to_path_buf())
        .unwrap_or_else(|| std::env::temp_dir().join("ferrum"));
    let _ = std::fs::create_dir_all(&dir);
    dir
}

pub fn vault_path() -> PathBuf {
    data_dir().join("vault.ferrum")
}

pub fn default_log_dir() -> PathBuf {
    let d = data_dir().join("logs");
    let _ = std::fs::create_dir_all(&d);
    d
}

fn device_key_path() -> PathBuf {
    data_dir().join("device.key")
}

fn device_key() -> Result<[u8; 32]> {
    let p = device_key_path();
    if let Ok(bytes) = std::fs::read(&p) {
        if bytes.len() == 32 {
            let mut k = [0u8; 32];
            k.copy_from_slice(&bytes);
            return Ok(k);
        }
    }
    let mut k = [0u8; 32];
    rand::rng().fill_bytes(&mut k);
    std::fs::write(&p, k).context("writing device key")?;
    restrict_permissions(&p);
    Ok(k)
}

#[cfg(unix)]
fn restrict_permissions(p: &Path) {
    use std::os::unix::fs::PermissionsExt;
    let _ = std::fs::set_permissions(p, std::fs::Permissions::from_mode(0o600));
}

#[cfg(not(unix))]
fn restrict_permissions(_p: &Path) {}

pub fn derive_key(password: &str, salt: &[u8]) -> Result<[u8; 32]> {
    let mut out = [0u8; 32];
    argon2::Argon2::default()
        .hash_password_into(password.as_bytes(), salt, &mut out)
        .map_err(|e| anyhow!("key derivation failed: {e}"))?;
    Ok(out)
}

pub fn encrypt_blob(plain: &[u8], password: Option<&str>) -> Result<Vec<u8>> {
    let mut salt = [0u8; 16];
    let mut nonce = [0u8; 24];
    rand::rng().fill_bytes(&mut salt);
    rand::rng().fill_bytes(&mut nonce);
    let (mode, key) = match password {
        Some(p) => (1u8, derive_key(p, &salt)?),
        None => (0u8, device_key()?),
    };
    let cipher = XChaCha20Poly1305::new_from_slice(&key).map_err(|_| anyhow!("bad key"))?;
    let ct = cipher
        .encrypt(&nonce.into(), plain)
        .map_err(|_| anyhow!("encryption failed"))?;
    let mut out = Vec::with_capacity(ct.len() + 48);
    out.extend_from_slice(MAGIC);
    out.push(mode);
    out.extend_from_slice(&salt);
    out.extend_from_slice(&nonce);
    out.extend_from_slice(&ct);
    Ok(out)
}

/// Returns true when the blob is protected by a master password.
pub fn blob_needs_password(blob: &[u8]) -> bool {
    blob.len() > 8 && &blob[..7] == MAGIC && blob[7] == 1
}

pub fn decrypt_blob(blob: &[u8], password: Option<&str>) -> Result<Vec<u8>> {
    if blob.len() < 7 + 1 + 16 + 24 + 16 || &blob[..7] != MAGIC {
        bail!("not a Ferrum vault file");
    }
    let mode = blob[7];
    let salt = &blob[8..24];
    let mut nonce = [0u8; 24];
    nonce.copy_from_slice(&blob[24..48]);
    let key = match mode {
        0 => device_key()?,
        1 => derive_key(
            password.ok_or_else(|| anyhow!("master password required"))?,
            salt,
        )?,
        _ => bail!("unknown vault mode"),
    };
    let cipher = XChaCha20Poly1305::new_from_slice(&key).map_err(|_| anyhow!("bad key"))?;
    cipher
        .decrypt(&nonce.into(), &blob[48..])
        .map_err(|_| anyhow!("wrong password or corrupted vault"))
}

pub struct Store {
    pub path: PathBuf,
    pub master_password: Option<String>,
}

pub enum LoadOutcome {
    Loaded(Vault),
    NeedsPassword,
    Fresh,
}

impl Store {
    pub fn new() -> Self {
        Self {
            path: vault_path(),
            master_password: None,
        }
    }

    pub fn probe(&self) -> Result<LoadOutcome> {
        let Ok(blob) = std::fs::read(&self.path) else {
            return Ok(LoadOutcome::Fresh);
        };
        if blob_needs_password(&blob) && self.master_password.is_none() {
            return Ok(LoadOutcome::NeedsPassword);
        }
        let plain = decrypt_blob(&blob, self.master_password.as_deref())?;
        let vault: Vault = serde_json::from_slice(&plain).context("parsing vault")?;
        Ok(LoadOutcome::Loaded(vault))
    }

    pub fn save(&self, vault: &Vault) -> Result<()> {
        let json = serde_json::to_vec(vault)?;
        let blob = encrypt_blob(&json, self.master_password.as_deref())?;
        let tmp = self.path.with_extension("tmp");
        std::fs::write(&tmp, &blob)?;
        restrict_permissions(&tmp);
        std::fs::rename(&tmp, &self.path)?;
        // Rolling backup.
        let _ = std::fs::write(self.path.with_extension("bak"), &blob);
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Sync
// ---------------------------------------------------------------------------

pub fn sync_file(folder: &str) -> PathBuf {
    Path::new(folder).join("ferrum-vault.sync")
}

/// Merge `remote` into `local` using per-entity last-writer-wins and tombstones.
pub fn merge_vaults(local: &mut Vault, remote: &Vault) {
    // Tombstones first.
    for (id, ts) in &remote.tombstones {
        let e = local.tombstones.entry(*id).or_insert(*ts);
        if *ts > *e {
            *e = *ts;
        }
    }
    macro_rules! merge {
        ($field:ident) => {{
            for r in &remote.$field {
                match local.$field.iter_mut().find(|l| l.id == r.id) {
                    Some(l) => {
                        if r.updated > l.updated {
                            *l = r.clone();
                        }
                    }
                    None => local.$field.push(r.clone()),
                }
            }
            let tomb = local.tombstones.clone();
            local
                .$field
                .retain(|x| tomb.get(&x.id).map(|t| *t < x.updated).unwrap_or(true));
        }};
    }
    merge!(hosts);
    merge!(groups);
    merge!(identities);
    merge!(keys);
    merge!(forwards);
    merge!(snippets);
    merge!(workspaces);
    merge!(known_hosts);
    for h in &remote.history {
        if !local
            .history
            .iter()
            .any(|l| l.id == h.id || l.command == h.command)
        {
            local.history.push(h.clone());
        }
    }
    local.history.sort_by_key(|h| h.timestamp);
    if remote.settings_updated > local.settings_updated {
        // Keep machine-local paths.
        let keep_sync = local.settings.sync_folder.clone();
        let keep_log = local.settings.log_dir.clone();
        let keep_shell = local.settings.local_shell.clone();
        local.settings = remote.settings.clone();
        local.settings.sync_folder = keep_sync;
        local.settings.log_dir = keep_log;
        local.settings.local_shell = keep_shell;
        local.settings_updated = remote.settings_updated;
    }
}

/// Pull the sync file (if present), merge, then push the merged vault back.
pub fn sync_with_folder(vault: &mut Vault, passphrase: &str) -> Result<String> {
    let folder = vault.settings.sync_folder.clone();
    if folder.trim().is_empty() {
        bail!("choose a sync folder in Settings → Sync first");
    }
    if passphrase.len() < 8 {
        bail!("sync passphrase must be at least 8 characters");
    }
    std::fs::create_dir_all(&folder).context("creating sync folder")?;
    let path = sync_file(&folder);
    let mut pulled = 0usize;
    if let Ok(blob) = std::fs::read(&path) {
        let plain = decrypt_blob(&blob, Some(passphrase))
            .context("decrypting sync file (wrong passphrase?)")?;
        let remote: Vault = serde_json::from_slice(&plain)?;
        pulled = remote.hosts.len();
        merge_vaults(vault, &remote);
    }
    let mut shared = vault.clone();
    shared.connection_logs.clear();
    let json = serde_json::to_vec(&shared)?;
    let blob = encrypt_blob(&json, Some(passphrase))?;
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, blob)?;
    std::fs::rename(&tmp, &path)?;
    Ok(format!(
        "Synced at {} ({} hosts locally, {} in remote copy)",
        chrono::Local::now().format("%H:%M:%S"),
        vault.hosts.len(),
        pulled
    ))
}

pub fn export_encrypted(vault: &Vault, path: &Path, passphrase: &str) -> Result<()> {
    let json = serde_json::to_vec_pretty(vault)?;
    std::fs::write(path, encrypt_blob(&json, Some(passphrase))?)?;
    Ok(())
}

pub fn import_encrypted(path: &Path, passphrase: &str) -> Result<Vault> {
    let blob = std::fs::read(path)?;
    let plain = decrypt_blob(&blob, Some(passphrase))?;
    Ok(serde_json::from_slice(&plain)?)
}

pub fn touch_settings(v: &mut Vault) {
    v.settings_updated = now_ts();
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Host;

    #[test]
    fn password_roundtrip_and_wrong_password() {
        let blob = encrypt_blob(b"secret data", Some("correct horse")).unwrap();
        assert!(blob_needs_password(&blob));
        assert_eq!(
            decrypt_blob(&blob, Some("correct horse")).unwrap(),
            b"secret data"
        );
        assert!(decrypt_blob(&blob, Some("wrong")).is_err());
    }

    #[test]
    fn merge_prefers_newer_and_respects_tombstones() {
        let mut a = Vault::default();
        let mut b = Vault::default();
        let mut h = Host::default();
        h.label = "old".into();
        h.updated = 10;
        a.hosts.push(h.clone());
        let mut h2 = h.clone();
        h2.label = "new".into();
        h2.updated = 20;
        b.hosts.push(h2);
        merge_vaults(&mut a, &b);
        assert_eq!(a.hosts[0].label, "new");

        let mut c = Vault::default();
        c.tombstones.insert(h.id, 30);
        merge_vaults(&mut a, &c);
        assert!(a.hosts.is_empty());
    }
}
