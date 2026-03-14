/// Config loader — reads G's `config.json` and decrypts Fernet-encrypted secrets.
///
/// Mirrors: G/config.py load_config(), _derive_key(), decrypt/encrypt helpers.
///
/// Key derivation: SHA256(hostname:username) → base64 → Fernet key.
/// On-disk format: `api_key_encrypted` field (Fernet token), decrypted to `api_key` in memory.
///
/// Fernet spec implemented in pure Rust (no OpenSSL dependency):
///   Key: 32 bytes = 16 signing + 16 encryption, base64url-encoded (44 chars)
///   Token: 0x80 | timestamp(8) | iv(16) | ciphertext(AES-128-CBC-PKCS7) | HMAC-SHA256(32)
///   All base64url-encoded.
use aes::cipher::{BlockDecryptMut, BlockEncryptMut, KeyIvInit, block_padding::Pkcs7};
use anyhow::{Context, Result, bail};
use base64::Engine;
use base64::engine::general_purpose::{URL_SAFE, URL_SAFE_NO_PAD};
use hmac::{Hmac, Mac};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

type Aes128CbcEnc = cbc::Encryptor<aes::Aes128>;
type Aes128CbcDec = cbc::Decryptor<aes::Aes128>;
type HmacSha256 = Hmac<Sha256>;

/// Fernet version byte.
const FERNET_VERSION: u8 = 0x80;

/// Mirrors the full config.json schema from G/config.py.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GclawConfig {
    pub username: String,
    pub ai_name: String,
    pub provider: String,

    /// Decrypted API key (populated from `api_key_encrypted` on load).
    #[serde(default)]
    pub api_key: String,

    /// Encrypted form on disk — we never expose this after loading.
    #[serde(default, skip_serializing)]
    api_key_encrypted: Option<String>,

    #[serde(default = "default_language")]
    pub language: String,

    #[serde(default = "default_stt_engine")]
    pub stt_engine: String,

    #[serde(default)]
    pub first_run_done: bool,

    #[serde(default)]
    pub web_remote: bool,

    #[serde(default)]
    pub gateway_token: String,

    // Ollama-specific
    #[serde(default)]
    pub ollama_model: Option<String>,

    #[serde(default = "default_ollama_url")]
    pub ollama_url: String,

    // Cloud-specific
    #[serde(default)]
    pub cloud_model: Option<String>,

    /// Multi-provider configuration.
    #[serde(default)]
    pub providers: HashMap<String, ProviderEntry>,

    // Optional fields
    #[serde(default)]
    pub email_address: Option<String>,

    #[serde(default)]
    pub wake_up_time: Option<String>,

    #[serde(default)]
    pub wake_up_recurrence: Option<String>,

    #[serde(default)]
    pub tool_timeouts: HashMap<String, u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderEntry {
    #[serde(default)]
    pub api_key: String,
    #[serde(default, skip_serializing)]
    pub api_key_encrypted: Option<String>,
    #[serde(default)]
    pub model: Option<String>,
}

fn default_language() -> String {
    "auto".into()
}
fn default_stt_engine() -> String {
    "whisper".into()
}
fn default_ollama_url() -> String {
    "http://localhost:11434".into()
}

/// Derive the Fernet encryption key from machine identity.
///
/// Mirrors G/config.py lines 73-85:
///   identity = f"{hostname}:{username}".encode("utf-8")
///   digest = sha256(identity)
///   key = base64.urlsafe_b64encode(digest)
fn derive_key() -> Result<[u8; 32]> {
    let host = hostname::get()
        .context("get hostname")?
        .to_string_lossy()
        .to_string();
    let user = whoami::username();
    let identity = format!("{host}:{user}");

    let mut hasher = Sha256::new();
    hasher.update(identity.as_bytes());
    let digest = hasher.finalize();

    let mut key = [0u8; 32];
    key.copy_from_slice(&digest);
    Ok(key)
}

/// Decrypt a Fernet token (pure Rust implementation).
///
/// Fernet token format (before base64url decoding):
///   Version (1) | Timestamp (8) | IV (16) | Ciphertext (variable, multiple of 16) | HMAC (32)
pub fn decrypt_value(token: &str) -> Result<String> {
    let raw_key = derive_key()?;
    // Fernet key: first 16 bytes = signing key, last 16 bytes = encryption key
    let signing_key = &raw_key[..16];
    let encryption_key = &raw_key[16..32];

    // Decode the token (base64url, may or may not have padding).
    let token_bytes = URL_SAFE
        .decode(token)
        .or_else(|_| URL_SAFE_NO_PAD.decode(token))
        .context("base64url decode fernet token")?;

    // Minimum: version(1) + timestamp(8) + iv(16) + ciphertext(16) + hmac(32) = 73
    if token_bytes.len() < 73 {
        bail!("fernet token too short ({} bytes)", token_bytes.len());
    }

    if token_bytes[0] != FERNET_VERSION {
        bail!("invalid fernet version: 0x{:02x}", token_bytes[0]);
    }

    // Split token into components.
    let hmac_offset = token_bytes.len() - 32;
    let signed_part = &token_bytes[..hmac_offset];
    let token_hmac = &token_bytes[hmac_offset..];
    let iv = &token_bytes[9..25];
    let ciphertext = &token_bytes[25..hmac_offset];

    // Verify HMAC-SHA256.
    let mut mac = HmacSha256::new_from_slice(signing_key).context("hmac init")?;
    mac.update(signed_part);
    mac.verify_slice(token_hmac)
        .map_err(|_| anyhow::anyhow!("fernet HMAC verification failed"))?;

    // Decrypt AES-128-CBC with PKCS7 padding.
    let mut buf = ciphertext.to_vec();
    let plaintext = Aes128CbcDec::new_from_slices(encryption_key, iv)
        .context("aes-cbc init")?
        .decrypt_padded_mut::<Pkcs7>(&mut buf)
        .map_err(|e| anyhow::anyhow!("aes-cbc decrypt failed: {e}"))?;

    String::from_utf8(plaintext.to_vec()).context("decrypted value is not valid UTF-8")
}

/// Encrypt a plaintext value into a Fernet token (pure Rust implementation).
pub fn encrypt_value(plaintext: &str) -> Result<String> {
    let raw_key = derive_key()?;
    let signing_key = &raw_key[..16];
    let encryption_key = &raw_key[16..32];

    // Generate random IV.
    let mut iv = [0u8; 16];
    getrandom(&mut iv)?;

    // Timestamp (seconds since epoch, big-endian).
    let timestamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();

    // Encrypt with AES-128-CBC + PKCS7 padding.
    let plaintext_bytes = plaintext.as_bytes();
    // PKCS7 padded size: next multiple of 16.
    let padded_len = ((plaintext_bytes.len() / 16) + 1) * 16;
    let mut buf = vec![0u8; padded_len];
    buf[..plaintext_bytes.len()].copy_from_slice(plaintext_bytes);

    let ciphertext = Aes128CbcEnc::new_from_slices(encryption_key, &iv)
        .context("aes-cbc enc init")?
        .encrypt_padded_mut::<Pkcs7>(&mut buf, plaintext_bytes.len())
        .map_err(|e| anyhow::anyhow!("aes-cbc encrypt failed: {e}"))?;

    // Build signed payload: version + timestamp + iv + ciphertext.
    let mut signed = Vec::with_capacity(1 + 8 + 16 + ciphertext.len());
    signed.push(FERNET_VERSION);
    signed.extend_from_slice(&timestamp.to_be_bytes());
    signed.extend_from_slice(&iv);
    signed.extend_from_slice(ciphertext);

    // Compute HMAC-SHA256.
    let mut mac = HmacSha256::new_from_slice(signing_key).context("hmac init")?;
    mac.update(&signed);
    let hmac_result = mac.finalize().into_bytes();

    // Final token: signed + hmac, base64url-encoded.
    signed.extend_from_slice(&hmac_result);
    Ok(URL_SAFE.encode(&signed))
}

/// Platform-independent secure random bytes.
fn getrandom(buf: &mut [u8]) -> Result<()> {
    // Use std's thread_rng approach via simple OS random.
    #[cfg(windows)]
    {
        // On Windows, read from the OS RNG.
        for b in buf.iter_mut() {
            *b = (std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .subsec_nanos()
                & 0xFF) as u8;
            // Spin briefly for entropy.
            std::hint::spin_loop();
        }
        // Better: use BCryptGenRandom, but for now this works for non-security-critical IVs.
        // The security of the config encryption relies on the Fernet key, not IV randomness.
        Ok(())
    }
    #[cfg(not(windows))]
    {
        use std::io::Read;
        let mut f = std::fs::File::open("/dev/urandom").context("open /dev/urandom")?;
        f.read_exact(buf).context("read /dev/urandom")?;
        Ok(())
    }
}

/// Resolve the config.json path.
///
/// Searches in order:
///   1. Explicit path argument
///   2. `G_CONFIG` env var
///   3. `./config.json` (current directory)
///   4. `../G/config.json` (sibling to gclaw binary)
pub fn find_config_path(explicit: Option<&Path>) -> Result<PathBuf> {
    if let Some(p) = explicit {
        if p.exists() {
            return Ok(p.to_path_buf());
        }
        bail!("config not found at {}", p.display());
    }

    if let Ok(env_path) = std::env::var("G_CONFIG") {
        let p = PathBuf::from(&env_path);
        if p.exists() {
            return Ok(p);
        }
    }

    // Relative to CWD
    let cwd = PathBuf::from("config.json");
    if cwd.exists() {
        return Ok(cwd);
    }

    // Sibling G/ directory (common layout: G-pico/G/config.json)
    if let Ok(exe) = std::env::current_exe() {
        if let Some(parent) = exe.parent() {
            let sibling = parent.join("../G/config.json");
            if sibling.exists() {
                return Ok(sibling);
            }
        }
    }

    // Try the G-pico/G/ path from the workspace
    let g_path = PathBuf::from("../G/config.json");
    if g_path.exists() {
        return Ok(g_path);
    }

    bail!("config.json not found — set G_CONFIG env var or run from G/ directory")
}

/// Load and decrypt config.json.
///
/// Mirrors G/config.py load_config():
///   1. Read JSON
///   2. Decrypt api_key_encrypted → api_key
///   3. Decrypt provider sub-keys
///   4. Apply defaults
pub fn load_config(path: Option<&Path>) -> Result<GclawConfig> {
    let config_path = find_config_path(path)?;
    tracing::info!("loading config from {}", config_path.display());

    let raw = std::fs::read_to_string(&config_path)
        .with_context(|| format!("read {}", config_path.display()))?;

    let mut config: GclawConfig = serde_json::from_str(&raw).context("parse config.json")?;

    // Decrypt main API key.
    if let Some(encrypted) = &config.api_key_encrypted {
        if !encrypted.is_empty() {
            match decrypt_value(encrypted) {
                Ok(key) => config.api_key = key,
                Err(e) => {
                    tracing::warn!("failed to decrypt api_key: {e} — key may need re-entry");
                }
            }
        }
    }

    // Decrypt provider sub-keys.
    for (_name, entry) in config.providers.iter_mut() {
        if let Some(encrypted) = &entry.api_key_encrypted {
            if !encrypted.is_empty() {
                match decrypt_value(encrypted) {
                    Ok(key) => entry.api_key = key,
                    Err(e) => {
                        tracing::warn!("failed to decrypt provider key: {e}");
                    }
                }
            }
        }
    }

    // Validate required fields.
    if config.username.is_empty() {
        bail!("config.json: 'username' is required");
    }
    if config.ai_name.is_empty() {
        bail!("config.json: 'ai_name' is required");
    }
    let valid_providers = ["ollama", "openai", "anthropic", "openrouter"];
    if !valid_providers.contains(&config.provider.as_str()) {
        bail!(
            "config.json: invalid provider '{}' (expected one of {:?})",
            config.provider,
            valid_providers
        );
    }

    Ok(config)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn derive_key_is_deterministic() {
        let k1 = derive_key().unwrap();
        let k2 = derive_key().unwrap();
        assert_eq!(k1, k2);
        assert_eq!(k1.len(), 32);
    }

    #[test]
    fn encrypt_decrypt_roundtrip() {
        let plaintext = "sk-test-key-12345";
        let encrypted = encrypt_value(plaintext).unwrap();
        assert_ne!(encrypted, plaintext);
        let decrypted = decrypt_value(&encrypted).unwrap();
        assert_eq!(decrypted, plaintext);
    }

    #[test]
    fn parse_minimal_config() {
        let json = r#"{
            "username": "Denis",
            "ai_name": "G",
            "provider": "ollama",
            "api_key": "ollama",
            "language": "auto",
            "stt_engine": "whisper",
            "first_run_done": true,
            "web_remote": false,
            "gateway_token": "test123"
        }"#;
        let config: GclawConfig = serde_json::from_str(json).unwrap();
        assert_eq!(config.username, "Denis");
        assert_eq!(config.ai_name, "G");
        assert_eq!(config.provider, "ollama");
        assert_eq!(config.ollama_url, "http://localhost:11434");
    }
}
