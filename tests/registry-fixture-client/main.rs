//! Verify the exact CLI-generated fixture using TikTools' registry client.
use std::{
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, RwLock,
    },
};
use tiktools_plugin_package::{
    KeyId, KeyRole, PackageArchive, Sha256Digest, SigningKey, TrustStore, TrustedKey,
};
use tiktools_plugin_registry::{
    join_url, DownloadRequest, Downloader, RegistryClient, RegistryError, RegistrySource,
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args_os().collect();
    if args.len() != 4 {
        return Err(
            "expected registry directory, separately supplied root pin, and scratch directory"
                .into(),
        );
    }
    let root = PathBuf::from(&args[1]);
    let pin: TrustedKey = serde_json::from_slice(&std::fs::read(&args[2])?)?;
    let scratch = PathBuf::from(&args[3]);
    assert_eq!(pin.role, KeyRole::Root);
    let tamper = Arc::new(AtomicBool::new(false));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let base = format!("http://{}", listener.local_addr()?);
    let server_tamper = tamper.clone();
    let server = tokio::spawn(async move {
        loop {
            let Ok((mut socket, _)) = listener.accept().await else {
                break;
            };
            let root = root.clone();
            let tamper = server_tamper.clone();
            tokio::spawn(async move {
                let mut buffer = [0; 16384];
                let Ok(n) = socket.read(&mut buffer).await else {
                    return;
                };
                let request = String::from_utf8_lossy(&buffer[..n]);
                let path = request
                    .lines()
                    .next()
                    .and_then(|s| s.split_whitespace().nth(1))
                    .unwrap_or("")
                    .split('?')
                    .next()
                    .unwrap_or("")
                    .trim_start_matches('/');
                if path.split('/').any(|part| part == ".." || part == ".") {
                    return;
                }
                let (status, mut body) = match std::fs::read(root.join(path)) {
                    Ok(body) => ("200 OK", body),
                    Err(_) => ("404 Not Found", Vec::new()),
                };
                if tamper.load(Ordering::SeqCst) && path == "index/ac/me/acme.hooks.json" {
                    body.push(b' ');
                }
                let header = format!(
                    "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                );
                let _ = socket.write_all(header.as_bytes()).await;
                let _ = socket.write_all(&body).await;
            });
        }
    });
    let make_client = |name: &str, key| -> Result<RegistryClient, Box<dyn std::error::Error>> {
        let mut source = RegistrySource::new("fixture", &base)?;
        source.root_keys = vec![key];
        Ok(RegistryClient::new(
            scratch.join(name),
            scratch.join(format!("{name}.json")),
            vec![source],
            Arc::new(RwLock::new(TrustStore::new())),
        )?)
    };
    let client = make_client("valid", pin.key.clone())?;
    assert!(client.refresh("fixture").await?.verified);
    let (source, document) = client.package("acme.hooks", None).await?;
    let artifact = &document
        .releases
        .iter()
        .find(|r| r.version == "0.1.0")
        .ok_or("missing release")?
        .artifacts[0];
    let url = join_url(&source.url, &artifact.url)?;
    let downloaded = Downloader::new(scratch.join("downloads"))?
        .fetch(
            DownloadRequest {
                url: &url,
                expected_digest: Some(&artifact.sha256),
                expected_size: Some(artifact.size),
            },
            &|_, _| {},
            &AtomicBool::new(false),
        )
        .await?;
    let archive = PackageArchive::open(&downloaded.path)?;
    let inspection = archive.inspect()?;
    assert_eq!(inspection.manifest.id, "acme.hooks");
    assert_eq!(inspection.manifest.version.to_string(), "0.1.0");
    let verified = client.trust().read().unwrap().verify_package(
        inspection.signature.as_ref().ok_or("unsigned package")?,
        inspection.checksums.as_ref().ok_or("missing checksums")?,
        "acme.hooks",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_secs(),
    )?;
    assert_eq!(verified.publisher, "acme");
    archive.extract(&scratch.join("extracted"))?;
    let wrong_digest = Sha256Digest::of_bytes(b"not the artifact");
    let rejected = Downloader::new(scratch.join("bad-download"))?
        .fetch(
            DownloadRequest {
                url: &url,
                expected_digest: Some(&wrong_digest),
                expected_size: Some(artifact.size),
            },
            &|_, _| {},
            &AtomicBool::new(false),
        )
        .await;
    assert!(
        matches!(rejected, Err(RegistryError::DigestMismatch { .. })),
        "wrong digest accepted or failed for another reason"
    );
    let other = SigningKey::generate(KeyId::parse("wrong-root")?, None)?;
    assert!(
        matches!(
            make_client("wrong-root", other.public_key())?
                .refresh("fixture")
                .await,
            Err(RegistryError::Signature { .. })
        ),
        "untrusted root accepted or failed for another reason"
    );
    tamper.store(true, Ordering::SeqCst);
    let tampered = make_client("tampered", pin.key.clone())?;
    assert!(
        matches!(
            tampered.package("acme.hooks", None).await,
            Err(RegistryError::Signature { .. })
        ),
        "tampered index accepted or failed for another reason"
    );
    server.abort();
    println!("fixture: pinned root, index, artifact digest and publisher signature verified; wrong root and tampering rejected");
    Ok(())
}
