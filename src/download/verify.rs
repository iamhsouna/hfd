use std::path::Path;

use anyhow::{Context, Result};
use sha2::{Digest, Sha256};
use tokio::io::AsyncReadExt;

/// Stream a file through SHA-256 and compare it with the expected hex digest.
pub async fn verify_sha256(path: &Path, expected: &str) -> Result<bool> {
    let file = tokio::fs::File::open(path)
        .await
        .with_context(|| format!("opening {} for verification", path.display()))?;
    let mut reader = tokio::io::BufReader::with_capacity(4 * 1024 * 1024, file);
    let mut hasher = Sha256::new();
    let mut buffer = vec![0u8; 4 * 1024 * 1024];
    loop {
        let read = reader.read(&mut buffer).await?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    let digest = hasher.finalize();
    Ok(hex(&digest).eq_ignore_ascii_case(expected.trim()))
}

fn hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(DIGITS[(byte >> 4) as usize] as char);
        out.push(DIGITS[(byte & 0x0f) as usize] as char);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn verifies_known_digest() {
        let dir = std::env::temp_dir().join("hfd-verify-test");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("hello.txt");
        std::fs::write(&path, b"hello").unwrap();
        let expected = "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824";
        assert!(verify_sha256(&path, expected).await.unwrap());
        assert!(!verify_sha256(&path, "deadbeef").await.unwrap());
        let _ = std::fs::remove_file(&path);
    }
}
