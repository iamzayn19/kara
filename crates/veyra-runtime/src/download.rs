//! Verified, resumable downloads.
//!
//! Data streams to `<dest>.part`. An interrupted download resumes with an
//! HTTP Range request; the existing bytes are re-hashed first so the final
//! digest covers the whole file. The file is renamed into place only after the
//! SHA-256 matches. A mismatch deletes the partial file.

use futures_util::StreamExt;
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use tokio::io::AsyncWriteExt;
use tokio_util::sync::CancellationToken;

#[derive(Debug, Clone, Copy)]
pub struct Progress {
    pub downloaded: u64,
    pub total: Option<u64>,
}

#[derive(Debug)]
pub enum DownloadError {
    Cancelled,
    Checksum { expected: String, actual: String },
    Http(String),
    Io(String),
}

impl std::fmt::Display for DownloadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DownloadError::Cancelled => {
                write!(f, "download cancelled (partial file kept for resume)")
            }
            DownloadError::Checksum { expected, actual } => write!(
                f,
                "checksum mismatch: expected sha256 {expected}, got {actual}. The file was deleted."
            ),
            DownloadError::Http(e) => write!(f, "download failed: {e}"),
            DownloadError::Io(e) => write!(f, "file error: {e}"),
        }
    }
}

impl std::error::Error for DownloadError {}

impl From<std::io::Error> for DownloadError {
    fn from(e: std::io::Error) -> Self {
        DownloadError::Io(e.to_string())
    }
}

pub fn part_path(dest: &Path) -> PathBuf {
    let mut s = dest.as_os_str().to_owned();
    s.push(".part");
    PathBuf::from(s)
}

pub fn sha256_file(path: &Path) -> std::io::Result<String> {
    use std::io::Read;
    let mut f = std::fs::File::open(path)?;
    let mut h = Sha256::new();
    let mut buf = vec![0u8; 1 << 20];
    loop {
        let n = f.read(&mut buf)?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
    }
    Ok(hex::encode(h.finalize()))
}

/// Download `url` to `dest`, verifying `sha256` when given.
pub async fn download_verified(
    client: &reqwest::Client,
    url: &str,
    dest: &Path,
    sha256: Option<&str>,
    on_progress: &(dyn Fn(Progress) + Send + Sync),
    cancel: &CancellationToken,
) -> Result<(), DownloadError> {
    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let part = part_path(dest);
    let mut hasher = Sha256::new();
    let mut have: u64 = 0;
    if part.exists() {
        // Hash what we already have so the final digest covers the whole file.
        use std::io::Read;
        let mut f = std::fs::File::open(&part)?;
        let mut buf = vec![0u8; 1 << 20];
        loop {
            let n = f.read(&mut buf)?;
            if n == 0 {
                break;
            }
            hasher.update(&buf[..n]);
            have += n as u64;
        }
    }

    let mut req = client
        .get(url)
        .header(reqwest::header::USER_AGENT, crate::USER_AGENT);
    if have > 0 {
        req = req.header(reqwest::header::RANGE, format!("bytes={have}-"));
    }
    let resp = req
        .send()
        .await
        .map_err(|e| DownloadError::Http(e.to_string()))?;
    let status = resp.status();
    if status == reqwest::StatusCode::RANGE_NOT_SATISFIABLE && have > 0 {
        // Already complete; fall through to verification.
    } else if !status.is_success() {
        return Err(DownloadError::Http(format!("{url} returned {status}")));
    }

    let resumed = status == reqwest::StatusCode::PARTIAL_CONTENT;
    if have > 0 && !resumed && status.is_success() {
        // Server ignored the range: start over.
        hasher = Sha256::new();
        have = 0;
    }
    let total = resp.content_length().map(|l| l + have);

    if status.is_success() {
        let mut file = if resumed {
            tokio::fs::OpenOptions::new()
                .append(true)
                .open(&part)
                .await?
        } else {
            tokio::fs::File::create(&part).await?
        };
        let mut stream = resp.bytes_stream();
        let mut last_report = 0u64;
        loop {
            let next = tokio::select! {
                c = stream.next() => c,
                _ = cancel.cancelled() => {
                    file.flush().await?;
                    return Err(DownloadError::Cancelled);
                }
            };
            let Some(chunk) = next else { break };
            let chunk = chunk.map_err(|e| DownloadError::Http(e.to_string()))?;
            hasher.update(&chunk);
            file.write_all(&chunk).await?;
            have += chunk.len() as u64;
            if have - last_report >= 1 << 20 || Some(have) == total {
                on_progress(Progress {
                    downloaded: have,
                    total,
                });
                last_report = have;
            }
        }
        file.flush().await?;
        drop(file);
    }

    let actual = hex::encode(hasher.finalize());
    if let Some(expected) = sha256 {
        if !actual.eq_ignore_ascii_case(expected) {
            let _ = std::fs::remove_file(&part);
            return Err(DownloadError::Checksum {
                expected: expected.to_string(),
                actual,
            });
        }
    }
    std::fs::rename(&part, dest)?;
    on_progress(Progress {
        downloaded: have,
        total: Some(have),
    });
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    /// Serves `body`, honouring a `Range: bytes=N-` header.
    async fn serve(body: Vec<u8>, connections: usize) -> String {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            for _ in 0..connections {
                let (mut s, _) = listener.accept().await.unwrap();
                let mut buf = vec![0u8; 8192];
                let n = s.read(&mut buf).await.unwrap();
                let req = String::from_utf8_lossy(&buf[..n]).to_lowercase();
                let start: usize = req
                    .lines()
                    .find_map(|l| l.strip_prefix("range: bytes="))
                    .and_then(|r| {
                        r.trim_end_matches('-')
                            .trim()
                            .trim_end_matches('-')
                            .parse()
                            .ok()
                    })
                    .unwrap_or(0);
                let slice = &body[start.min(body.len())..];
                let head = if start > 0 {
                    format!(
                        "HTTP/1.1 206 Partial Content\r\ncontent-length: {}\r\n\r\n",
                        slice.len()
                    )
                } else {
                    format!("HTTP/1.1 200 OK\r\ncontent-length: {}\r\n\r\n", slice.len())
                };
                s.write_all(head.as_bytes()).await.unwrap();
                s.write_all(slice).await.unwrap();
            }
        });
        format!("http://{addr}/file")
    }

    fn sha(b: &[u8]) -> String {
        hex::encode(Sha256::digest(b))
    }

    #[tokio::test]
    async fn downloads_and_verifies() {
        let body: Vec<u8> = (0..300_000u32).map(|i| (i % 251) as u8).collect();
        let url = serve(body.clone(), 1).await;
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("m.gguf");
        let client = reqwest::Client::new();
        download_verified(
            &client,
            &url,
            &dest,
            Some(&sha(&body)),
            &|_| {},
            &CancellationToken::new(),
        )
        .await
        .unwrap();
        assert_eq!(std::fs::read(&dest).unwrap(), body);
        assert!(!part_path(&dest).exists());
    }

    #[tokio::test]
    async fn checksum_mismatch_deletes_partial() {
        let url = serve(b"tampered".to_vec(), 1).await;
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("m.gguf");
        let err = download_verified(
            &reqwest::Client::new(),
            &url,
            &dest,
            Some(&sha(b"original")),
            &|_| {},
            &CancellationToken::new(),
        )
        .await
        .unwrap_err();
        assert!(matches!(err, DownloadError::Checksum { .. }));
        assert!(!dest.exists());
        assert!(!part_path(&dest).exists());
    }

    #[tokio::test]
    async fn resumes_partial_download() {
        let body: Vec<u8> = (0..100_000u32).map(|i| (i % 13) as u8).collect();
        let url = serve(body.clone(), 1).await;
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("m.gguf");
        std::fs::write(part_path(&dest), &body[..40_000]).unwrap();
        download_verified(
            &reqwest::Client::new(),
            &url,
            &dest,
            Some(&sha(&body)),
            &|_| {},
            &CancellationToken::new(),
        )
        .await
        .unwrap();
        assert_eq!(std::fs::read(&dest).unwrap(), body);
    }
}
