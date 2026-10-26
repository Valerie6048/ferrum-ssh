//! SSH key generation, import and inspection.

use anyhow::{Result, anyhow, bail};
use ssh_key::{
    Algorithm, EcdsaCurve, HashAlg, LineEnding, PrivateKey, getrandom::SysRng, rand_core::UnwrapErr,
};

use crate::model::{SshKey, now_ts};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KeyKind {
    Ed25519,
    EcdsaP256,
    EcdsaP384,
    EcdsaP521,
    Rsa2048,
    Rsa3072,
    Rsa4096,
}

impl KeyKind {
    pub const ALL: [KeyKind; 7] = [
        KeyKind::Ed25519,
        KeyKind::EcdsaP256,
        KeyKind::EcdsaP384,
        KeyKind::EcdsaP521,
        KeyKind::Rsa2048,
        KeyKind::Rsa3072,
        KeyKind::Rsa4096,
    ];
    pub fn label(&self) -> &'static str {
        match self {
            KeyKind::Ed25519 => "ED25519 (recommended)",
            KeyKind::EcdsaP256 => "ECDSA P-256",
            KeyKind::EcdsaP384 => "ECDSA P-384",
            KeyKind::EcdsaP521 => "ECDSA P-521",
            KeyKind::Rsa2048 => "RSA 2048",
            KeyKind::Rsa3072 => "RSA 3072",
            KeyKind::Rsa4096 => "RSA 4096",
        }
    }
}

pub fn generate(kind: KeyKind, label: &str, passphrase: &str) -> Result<SshKey> {
    let mut rng = UnwrapErr(SysRng);
    let mut key = match kind {
        KeyKind::Ed25519 => PrivateKey::random(&mut rng, Algorithm::Ed25519)?,
        KeyKind::EcdsaP256 => PrivateKey::random(
            &mut rng,
            Algorithm::Ecdsa {
                curve: EcdsaCurve::NistP256,
            },
        )?,
        KeyKind::EcdsaP384 => PrivateKey::random(
            &mut rng,
            Algorithm::Ecdsa {
                curve: EcdsaCurve::NistP384,
            },
        )?,
        KeyKind::EcdsaP521 => PrivateKey::random(
            &mut rng,
            Algorithm::Ecdsa {
                curve: EcdsaCurve::NistP521,
            },
        )?,
        KeyKind::Rsa2048 | KeyKind::Rsa3072 | KeyKind::Rsa4096 => {
            let bits = match kind {
                KeyKind::Rsa2048 => 2048,
                KeyKind::Rsa3072 => 3072,
                _ => 4096,
            };
            let kp = ssh_key::private::RsaKeypair::random(&mut rng, bits)?;
            PrivateKey::from(kp)
        }
    };
    key.set_comment(label);
    let public = key.public_key().to_openssh()?;
    let private = if passphrase.is_empty() {
        key.to_openssh(LineEnding::LF)?.to_string()
    } else {
        key.encrypt(&mut rng, passphrase)?
            .to_openssh(LineEnding::LF)?
            .to_string()
    };
    Ok(SshKey {
        id: uuid::Uuid::new_v4(),
        label: label.to_string(),
        private_key: private,
        public_key: public,
        passphrase: passphrase.to_string(),
        certificate: String::new(),
        key_type: key.algorithm().as_str().to_string(),
        updated: now_ts(),
    })
}

/// Build a vault key from pasted/imported private key text (OpenSSH or PEM/PPK).
pub fn import(label: &str, private_text: &str, passphrase: &str) -> Result<SshKey> {
    let text = private_text.trim();
    if text.is_empty() {
        bail!("paste a private key");
    }
    let pass = (!passphrase.is_empty()).then_some(passphrase);
    let parsed = russh::keys::decode_secret_key(text, pass).map_err(|e| {
        anyhow!("could not read private key: {e}. If it is encrypted, enter its passphrase.")
    })?;
    let public = parsed.public_key().to_openssh().unwrap_or_default();
    let key_type = parsed.algorithm().as_str().to_string();
    Ok(SshKey {
        id: uuid::Uuid::new_v4(),
        label: if label.is_empty() {
            key_type.clone()
        } else {
            label.to_string()
        },
        private_key: text.to_string() + "\n",
        public_key: public,
        passphrase: passphrase.to_string(),
        certificate: String::new(),
        key_type,
        updated: now_ts(),
    })
}

pub fn fingerprint_of_public(public: &str) -> String {
    ssh_key::PublicKey::from_openssh(public.trim())
        .map(|k| k.fingerprint(HashAlg::Sha256).to_string())
        .unwrap_or_else(|_| "—".into())
}

/// Scan ~/.ssh for private keys to offer on first import.
pub fn discover_local_keys() -> Vec<(String, String)> {
    let Some(home) = directories::UserDirs::new().map(|u| u.home_dir().join(".ssh")) else {
        return vec![];
    };
    let mut out = vec![];
    if let Ok(rd) = std::fs::read_dir(home) {
        for e in rd.flatten() {
            let p = e.path();
            if p.extension().is_some() {
                continue;
            }
            if let Ok(text) = std::fs::read_to_string(&p) {
                if text.contains("PRIVATE KEY") {
                    out.push((p.file_name().unwrap().to_string_lossy().to_string(), text));
                }
            }
        }
    }
    out.sort();
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generate_and_reimport_ed25519_with_passphrase() {
        let k = generate(KeyKind::Ed25519, "test", "hunter22").unwrap();
        assert!(k.public_key.starts_with("ssh-ed25519 "));
        assert!(k.private_key.contains("OPENSSH PRIVATE KEY"));
        let again = import("x", &k.private_key, "hunter22").unwrap();
        assert_eq!(
            again.public_key.split_whitespace().nth(1),
            k.public_key.split_whitespace().nth(1)
        );
        assert!(import("x", &k.private_key, "wrong").is_err());
        assert!(fingerprint_of_public(&k.public_key).starts_with("SHA256:"));
    }

    #[test]
    fn generate_ecdsa() {
        let k = generate(KeyKind::EcdsaP256, "e", "").unwrap();
        assert!(k.public_key.starts_with("ecdsa-sha2-nistp256 "));
    }
}
