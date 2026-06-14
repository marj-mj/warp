#[cfg(not(target_os = "windows"))]
compile_error!("warp_gateway_wrapper currently supports Windows only.");

use std::fs;
use std::path::PathBuf;

use anyhow::{Context, Result};
use serde::{de::DeserializeOwned, Serialize};
use windows::core::BSTR;
use windows::Win32::Foundation::{LocalFree, HLOCAL};
use windows::Win32::Security::Cryptography::{
    CryptProtectData, CryptUnprotectData, CRYPT_INTEGER_BLOB,
};

pub struct WindowsSecureStorage {
    service_name: String,
    storage_dir: PathBuf,
}

impl WindowsSecureStorage {
    pub fn new(service_name: &str, storage_dir: PathBuf) -> Self {
        Self {
            service_name: service_name.to_string(),
            storage_dir,
        }
    }

    pub fn storage_file(&self, key: &str) -> PathBuf {
        let filename = format!("{}-{key}", self.service_name);
        self.storage_dir.join(filename)
    }

    pub fn read_json<T: DeserializeOwned>(&self, key: &str) -> Result<Option<T>> {
        let path = self.storage_file(key);
        if !path.exists() {
            return Ok(None);
        }

        let encrypted = fs::read(&path)
            .with_context(|| format!("failed to read secure storage file {}", path.display()))?;
        let plaintext = decrypt(encrypted)?;
        let value = serde_json::from_str(&plaintext)
            .with_context(|| format!("failed to deserialize JSON from {}", path.display()))?;
        Ok(Some(value))
    }

    pub fn write_json<T: Serialize>(&self, key: &str, value: &T) -> Result<()> {
        fs::create_dir_all(&self.storage_dir).with_context(|| {
            format!(
                "failed to create secure storage directory {}",
                self.storage_dir.display()
            )
        })?;
        let path = self.storage_file(key);
        let plaintext = serde_json::to_string(value).context("failed to serialize JSON")?;
        let encrypted = encrypt(key, plaintext)?;
        fs::write(&path, encrypted)
            .with_context(|| format!("failed to write secure storage file {}", path.display()))?;
        Ok(())
    }
}

fn byte_vec_to_blob(bytes: &mut Vec<u8>) -> CRYPT_INTEGER_BLOB {
    let slice = bytes.as_mut_slice();
    CRYPT_INTEGER_BLOB {
        cbData: slice.len() as u32,
        pbData: slice.as_mut_ptr(),
    }
}

fn encrypt(key: &str, mut plaintext: String) -> Result<Vec<u8>> {
    let mut encrypted_blob = CRYPT_INTEGER_BLOB::default();
    let encrypted = unsafe {
        let plaintext_bytes = plaintext.as_bytes_mut();
        let plaintext_blob = CRYPT_INTEGER_BLOB {
            cbData: plaintext_bytes.len() as u32,
            pbData: plaintext_bytes.as_mut_ptr(),
        };
        CryptProtectData(
            &plaintext_blob,
            &BSTR::from(key),
            None,
            None,
            None,
            0,
            &mut encrypted_blob,
        )?;
        let output =
            std::slice::from_raw_parts(encrypted_blob.pbData, encrypted_blob.cbData as usize)
                .to_vec();
        LocalFree(Some(HLOCAL(encrypted_blob.pbData.cast())));
        output
    };

    Ok(encrypted)
}

fn decrypt(mut encrypted: Vec<u8>) -> Result<String> {
    let encrypted_blob = byte_vec_to_blob(&mut encrypted);
    let mut decrypted_blob = CRYPT_INTEGER_BLOB::default();

    let decrypted = unsafe {
        CryptUnprotectData(
            &encrypted_blob,
            None,
            None,
            None,
            None,
            0,
            &mut decrypted_blob,
        )?;
        let output =
            std::slice::from_raw_parts(decrypted_blob.pbData, decrypted_blob.cbData as usize)
                .to_vec();
        LocalFree(Some(HLOCAL(decrypted_blob.pbData.cast())));
        output
    };

    String::from_utf8(decrypted).context("secure storage payload was not valid UTF-8")
}
