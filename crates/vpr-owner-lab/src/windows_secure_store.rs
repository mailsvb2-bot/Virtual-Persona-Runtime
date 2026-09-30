use keyring::{Entry, Error as KeyringError};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

const STORE_SCHEMA: &str = "vpr-windows-chunked-secret-1";
const CHUNK_CHAR_LIMIT: usize = 512;
const MAX_CHUNKS: usize = 256;
const SLOT_A: &str = "a";
const SLOT_B: &str = "b";
const HEX: &[u8; 16] = b"0123456789abcdef";

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct RootManifest {
    schema_version: String,
    active_slot: String,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct SlotManifest {
    schema_version: String,
    state: String,
    chunk_count: usize,
    sha256: String,
}

pub(crate) struct ChunkedCredentialStore {
    service: &'static str,
    prefix: &'static str,
    legacy_account: Option<&'static str>,
}

impl ChunkedCredentialStore {
    pub(crate) const fn new(
        service: &'static str,
        prefix: &'static str,
        legacy_account: Option<&'static str>,
    ) -> Self {
        Self {
            service,
            prefix,
            legacy_account,
        }
    }

    pub(crate) fn load(&self) -> Result<Option<String>, String> {
        let Some(root) = self.read_root()? else {
            return self.load_legacy();
        };
        let slot = validate_slot(&root.active_slot)?;
        let manifest = self
            .read_slot_manifest(slot)?
            .ok_or_else(|| "Windows secure store manifest is incomplete".to_owned())?;
        validate_ready_manifest(&manifest)?;

        let mut raw = String::new();
        for index in 0..manifest.chunk_count {
            let account = self.chunk_account(slot, index);
            let chunk = self
                .read_optional(&account)?
                .ok_or_else(|| "Windows secure store chunk is missing".to_owned())?;
            raw.push_str(&chunk);
        }
        if sha256_hex(raw.as_bytes()) != manifest.sha256 {
            return Err("Windows secure store integrity check failed".into());
        }
        Ok(Some(raw))
    }

    pub(crate) fn save(&self, raw: &str) -> Result<(), String> {
        let chunks = split_chunks(raw)?;
        let root = self.read_root()?;
        let active_slot = root
            .as_ref()
            .map(|root| validate_slot(&root.active_slot))
            .transpose()?;
        if let Some(slot) = active_slot {
            let active_manifest = self
                .read_slot_manifest(slot)?
                .ok_or_else(|| "Windows secure store active manifest is missing".to_owned())?;
            validate_ready_manifest(&active_manifest)?;
        }

        let target_slot = match active_slot {
            Some(SLOT_A) => SLOT_B,
            _ => SLOT_A,
        };
        self.clear_slot(target_slot)?;

        let digest = sha256_hex(raw.as_bytes());
        self.write_slot_manifest(
            target_slot,
            &SlotManifest {
                schema_version: STORE_SCHEMA.into(),
                state: "staging".into(),
                chunk_count: chunks.len(),
                sha256: digest.clone(),
            },
        )?;
        for (index, chunk) in chunks.iter().enumerate() {
            self.write_password(&self.chunk_account(target_slot, index), chunk)?;
        }
        self.write_slot_manifest(
            target_slot,
            &SlotManifest {
                schema_version: STORE_SCHEMA.into(),
                state: "ready".into(),
                chunk_count: chunks.len(),
                sha256: digest,
            },
        )?;
        self.write_password(
            &self.root_account(),
            &serde_json::to_string(&RootManifest {
                schema_version: STORE_SCHEMA.into(),
                active_slot: target_slot.into(),
            })
            .map_err(|_| "Windows secure store root serialization failed")?,
        )?;

        if let Some(old_slot) = active_slot {
            let _ = self.clear_slot(old_slot);
        }
        if let Some(legacy) = self.legacy_account {
            let _ = self.delete_optional(legacy);
        }
        Ok(())
    }

    pub(crate) fn delete(&self) -> Result<(), String> {
        self.clear_slot(SLOT_A)?;
        self.clear_slot(SLOT_B)?;
        self.delete_optional(&self.root_account())?;
        if let Some(legacy) = self.legacy_account {
            self.delete_optional(legacy)?;
        }
        Ok(())
    }

    fn load_legacy(&self) -> Result<Option<String>, String> {
        match self.legacy_account {
            Some(account) => self.read_optional(account),
            None => Ok(None),
        }
    }

    fn read_root(&self) -> Result<Option<RootManifest>, String> {
        let Some(raw) = self.read_optional(&self.root_account())? else {
            return Ok(None);
        };
        let root: RootManifest = serde_json::from_str(&raw)
            .map_err(|_| "Windows secure store root manifest is invalid")?;
        if root.schema_version != STORE_SCHEMA {
            return Err("Windows secure store schema is unsupported".into());
        }
        validate_slot(&root.active_slot)?;
        Ok(Some(root))
    }

    fn read_slot_manifest(&self, slot: &str) -> Result<Option<SlotManifest>, String> {
        let Some(raw) = self.read_optional(&self.slot_manifest_account(slot))? else {
            return Ok(None);
        };
        let manifest: SlotManifest = serde_json::from_str(&raw)
            .map_err(|_| "Windows secure store slot manifest is invalid")?;
        validate_manifest(&manifest)?;
        Ok(Some(manifest))
    }

    fn write_slot_manifest(&self, slot: &str, manifest: &SlotManifest) -> Result<(), String> {
        let raw = serde_json::to_string(manifest)
            .map_err(|_| "Windows secure store slot manifest serialization failed")?;
        self.write_password(&self.slot_manifest_account(slot), &raw)
    }

    fn clear_slot(&self, slot: &str) -> Result<(), String> {
        let Some(manifest) = self.read_slot_manifest(slot)? else {
            return Ok(());
        };
        for index in 0..manifest.chunk_count {
            self.delete_optional(&self.chunk_account(slot, index))?;
        }
        self.delete_optional(&self.slot_manifest_account(slot))
    }

    fn read_optional(&self, account: &str) -> Result<Option<String>, String> {
        match self.entry(account)?.get_password() {
            Ok(value) => Ok(Some(value)),
            Err(KeyringError::NoEntry) => Ok(None),
            Err(_) => Err("Windows secure store read failed".into()),
        }
    }

    fn write_password(&self, account: &str, value: &str) -> Result<(), String> {
        self.entry(account)?
            .set_password(value)
            .map_err(|_| "Windows secure store write failed".to_owned())
    }

    fn delete_optional(&self, account: &str) -> Result<(), String> {
        match self.entry(account)?.delete_credential() {
            Ok(()) | Err(KeyringError::NoEntry) => Ok(()),
            Err(_) => Err("Windows secure store delete failed".into()),
        }
    }

    fn entry(&self, account: &str) -> Result<Entry, String> {
        Entry::new(self.service, account)
            .map_err(|_| "Windows secure store initialization failed".into())
    }

    fn root_account(&self) -> String {
        format!("{}-root", self.prefix)
    }

    fn slot_manifest_account(&self, slot: &str) -> String {
        format!("{}-{slot}-manifest", self.prefix)
    }

    fn chunk_account(&self, slot: &str, index: usize) -> String {
        format!("{}-{slot}-chunk-{index:03}", self.prefix)
    }
}

fn validate_slot(slot: &str) -> Result<&str, String> {
    match slot {
        SLOT_A => Ok(SLOT_A),
        SLOT_B => Ok(SLOT_B),
        _ => Err("Windows secure store slot is invalid".into()),
    }
}

fn validate_manifest(manifest: &SlotManifest) -> Result<(), String> {
    if manifest.schema_version != STORE_SCHEMA
        || !matches!(manifest.state.as_str(), "staging" | "ready")
        || manifest.chunk_count == 0
        || manifest.chunk_count > MAX_CHUNKS
        || manifest.sha256.len() != 64
        || !manifest.sha256.bytes().all(|byte| byte.is_ascii_hexdigit())
    {
        return Err("Windows secure store slot manifest is invalid".into());
    }
    Ok(())
}

fn validate_ready_manifest(manifest: &SlotManifest) -> Result<(), String> {
    validate_manifest(manifest)?;
    if manifest.state != "ready" {
        return Err("Windows secure store active slot is not committed".into());
    }
    Ok(())
}

fn split_chunks(raw: &str) -> Result<Vec<String>, String> {
    if raw.is_empty() {
        return Err("Windows secure store cannot persist an empty secret".into());
    }
    let mut chunks = Vec::new();
    let mut chunk = String::new();
    let mut chars = 0_usize;
    for value in raw.chars() {
        if chars == CHUNK_CHAR_LIMIT {
            chunks.push(std::mem::take(&mut chunk));
            chars = 0;
        }
        chunk.push(value);
        chars += 1;
    }
    if !chunk.is_empty() {
        chunks.push(chunk);
    }
    if chunks.is_empty() || chunks.len() > MAX_CHUNKS {
        return Err("Windows secure store value exceeds the bounded chunk capacity".into());
    }
    Ok(chunks)
}

fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut value = String::with_capacity(digest.len() * 2);
    for byte in digest {
        value.push(char::from(HEX[usize::from(byte >> 4)]));
        value.push(char::from(HEX[usize::from(byte & 0x0f)]));
    }
    value
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn unique_prefix(label: &str) -> String {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        format!("vpr-{label}-{}-{unique}", std::process::id())
    }

    #[test]
    fn chunk_split_stays_below_windows_unicode_blob_boundary() {
        let raw = "🦀".repeat(CHUNK_CHAR_LIMIT * 2 + 1);
        let chunks = split_chunks(&raw).unwrap();
        assert_eq!(chunks.len(), 3);
        assert!(
            chunks
                .iter()
                .all(|chunk| chunk.chars().count() <= CHUNK_CHAR_LIMIT)
        );
        assert_eq!(chunks.concat(), raw);
    }

    #[test]
    fn windows_chunk_store_round_trips_large_unicode_and_migrates_legacy() {
        let prefix = unique_prefix("chunk-store");
        let legacy = format!("{prefix}-legacy");
        let prefix: &'static str = Box::leak(prefix.into_boxed_str());
        let legacy: &'static str = Box::leak(legacy.into_boxed_str());
        let store =
            ChunkedCredentialStore::new("Virtual-Persona-Runtime-Test", prefix, Some(legacy));
        let _ = store.delete();

        store.write_password(legacy, "legacy-value").unwrap();
        assert_eq!(store.load().unwrap().as_deref(), Some("legacy-value"));

        let raw = format!("{}{}", "Ж".repeat(12_000), "🦀".repeat(1_000));
        store.save(&raw).unwrap();
        assert_eq!(store.load().unwrap().as_deref(), Some(raw.as_str()));
        assert_eq!(store.read_optional(legacy).unwrap(), None);

        store.delete().unwrap();
        assert_eq!(store.load().unwrap(), None);
    }
}
