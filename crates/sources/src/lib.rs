//! Source backends implementing [`iiif_core::source::ByteRangeSource`].
//!
//! M0 ships the local-filesystem backend; `object_store` backends arrive
//! at M4 through the same seam.

use bytes::Bytes;
use iiif_core::ident::Identifier;
use iiif_core::source::{BoxFuture, ByteRangeSource, SourceError};
use std::fs::File;
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// A local file opened for ranged reads. Reads happen on the blocking
/// thread pool; the file handle is shared and never mutated (seeks use
/// per-call `read_at`-style offsets via a cloned handle).
pub struct LocalFile {
    file: Arc<File>,
    len: u64,
    /// Modification time in whole seconds since the epoch — one half of
    /// the source-version pair the M5 `ETag` hashes.
    modified_secs: u64,
}

impl LocalFile {
    /// # Errors
    ///
    /// [`SourceError::NotFound`] when the path does not exist;
    /// [`SourceError::Io`] for any other open/stat failure.
    pub fn open(path: &Path) -> Result<Self, SourceError> {
        let file = File::open(path)?;
        let metadata = file.metadata().map_err(SourceError::from)?;
        let len = metadata.len();
        let modified_secs = metadata
            .modified()
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map_or(0, |d| d.as_secs());
        Ok(Self {
            file: Arc::new(file),
            len,
            modified_secs,
        })
    }

    /// The source-version pair (mtime seconds, byte length) that the `ETag`
    /// definition hashes: cheap, correct, no state.
    #[must_use]
    pub fn source_version(&self) -> (u64, u64) {
        (self.modified_secs, self.len)
    }
}

impl LocalFile {
    /// Surrender a plain `std::fs::File` for the sync decoder bridge.
    /// Falls back to `try_clone` when other handles are still alive.
    ///
    /// # Errors
    ///
    /// Propagates the `try_clone` failure in the shared-handle case.
    pub fn into_std_file(self) -> std::io::Result<File> {
        match Arc::try_unwrap(self.file) {
            Ok(file) => Ok(file),
            Err(shared) => shared.try_clone(),
        }
    }
}

impl ByteRangeSource for LocalFile {
    fn read_range(&self, offset: u64, len: u64) -> BoxFuture<'_, Result<Bytes, SourceError>> {
        let file = Arc::clone(&self.file);
        let source_len = self.len;
        Box::pin(async move {
            if offset.checked_add(len).is_none_or(|end| end > source_len) {
                return Err(SourceError::OutOfRange {
                    offset,
                    len,
                    source_len,
                });
            }
            let Ok(len_usize) = usize::try_from(len) else {
                return Err(SourceError::OutOfRange {
                    offset,
                    len,
                    source_len,
                });
            };
            let join = tokio::task::spawn_blocking(move || {
                let mut buf = vec![0u8; len_usize];
                read_exact_at(&file, &mut buf, offset)?;
                Ok::<_, std::io::Error>(Bytes::from(buf))
            })
            .await;
            match join {
                Ok(Ok(bytes)) => Ok(bytes),
                Ok(Err(e)) => Err(SourceError::from(e)),
                Err(e) => Err(SourceError::Io(std::io::Error::other(e))),
            }
        })
    }

    fn length(&self) -> BoxFuture<'_, Result<u64, SourceError>> {
        let len = self.len;
        Box::pin(async move { Ok(len) })
    }
}

/// Positional read without moving a shared cursor.
#[cfg(unix)]
fn read_exact_at(file: &File, buf: &mut [u8], offset: u64) -> std::io::Result<()> {
    use std::os::unix::fs::FileExt;
    file.read_exact_at(buf, offset)
}

/// Fallback for non-unix targets: clone the handle so the shared cursor is
/// untouched, then seek+read on the clone.
#[cfg(not(unix))]
fn read_exact_at(file: &File, buf: &mut [u8], offset: u64) -> std::io::Result<()> {
    use std::io::{Read, Seek, SeekFrom};
    let mut clone = file.try_clone()?;
    clone.seek(SeekFrom::Start(offset))?;
    clone.read_exact(buf)
}

/// Resolves identifiers against a filesystem root.
///
/// [`Identifier`] already guarantees no traversal segments; the canonical
/// containment check here is defense in depth (symlinks inside the tree
/// that point outside it are refused).
pub struct LocalRoot {
    root: PathBuf,
}

impl LocalRoot {
    /// # Errors
    ///
    /// Fails when the root does not exist or cannot be canonicalized.
    pub fn new(root: &Path) -> std::io::Result<Self> {
        Ok(Self {
            root: root.canonicalize()?,
        })
    }

    /// # Errors
    ///
    /// [`SourceError::NotFound`] when the identifier does not resolve to a
    /// file inside the root (including symlink escapes); [`SourceError::Io`]
    /// for other filesystem failures.
    pub fn resolve(&self, id: &Identifier) -> Result<LocalFile, SourceError> {
        let path = self.root.join(id.as_path());
        let canonical = path.canonicalize().map_err(SourceError::from)?;
        if !canonical.starts_with(&self.root) {
            return Err(SourceError::NotFound);
        }
        LocalFile::open(&canonical)
    }
}

/// Install the process-wide TLS crypto provider (ring), required before
/// any HTTPS object-store client is built. Idempotent; the explicit call
/// keeps the provider choice visible (see the Cargo.toml note and
/// docs/spikes/objstore-minio.md).
pub fn init_tls() {
    // A second call returns Err(already installed) — fine.
    let _ = rustls::crypto::ring::default_provider().install_default();
}

/// An S3-compatible object-store root: `s3://bucket/prefix` plus an
/// optional custom endpoint (Hetzner, `MinIO`, …). Masters are fetched
/// whole — the design spec's acknowledged model for JP2 (`&[u8]` input);
/// the bounded source-chunk/metadata cache is the recorded refinement.
pub struct ObjectRoot {
    store: std::sync::Arc<dyn object_store::ObjectStore>,
    prefix: String,
}

impl ObjectRoot {
    /// Build from an `s3://bucket[/prefix]` URL. Credentials and region
    /// come from the environment (the credential swamp is `object_store`’s
    /// job); `endpoint` overrides for S3-compatible services.
    ///
    /// # Errors
    ///
    /// Fails when the URL is not `s3://bucket[/prefix]` or the client
    /// cannot be constructed.
    pub fn new(url: &str, endpoint: Option<&str>) -> Result<Self, String> {
        let rest = url
            .strip_prefix("s3://")
            .ok_or_else(|| format!("not an s3:// URL: {url}"))?;
        let (bucket, prefix) = match rest.split_once('/') {
            Some((bucket, prefix)) => (bucket, prefix.trim_end_matches('/').to_owned()),
            None => (rest, String::new()),
        };
        if bucket.is_empty() {
            return Err("s3 URL has no bucket".to_owned());
        }
        let mut builder = object_store::aws::AmazonS3Builder::from_env()
            .with_bucket_name(bucket)
            .with_allow_http(true);
        if let Some(endpoint) = endpoint {
            builder = builder.with_endpoint(endpoint);
        }
        let store = builder.build().map_err(|e| format!("object store: {e}"))?;
        Ok(Self {
            store: std::sync::Arc::new(store),
            prefix,
        })
    }

    /// Fetch the master for `id` whole, returning its bytes and the
    /// source-version pair for the `ETag` (store `ETag` hashed + length).
    ///
    /// # Errors
    ///
    /// [`SourceError::NotFound`] for missing objects; [`SourceError::Io`]
    /// for transport failures.
    pub async fn resolve(&self, id: &Identifier) -> Result<(Bytes, (u64, u64)), SourceError> {
        use object_store::ObjectStoreExt;
        let path = if self.prefix.is_empty() {
            object_store::path::Path::from(id.as_path())
        } else {
            object_store::path::Path::from(format!("{}/{}", self.prefix, id.as_path()))
        };
        let result = self.store.get(&path).await.map_err(|e| match e {
            object_store::Error::NotFound { .. } => SourceError::NotFound,
            other => SourceError::Io(std::io::Error::other(other)),
        })?;
        let meta = result.meta.clone();
        let bytes = result
            .bytes()
            .await
            .map_err(|e| SourceError::Io(std::io::Error::other(e)))?;
        let version_hash = {
            use std::hash::{Hash, Hasher};
            let mut hasher = std::collections::hash_map::DefaultHasher::new();
            meta.e_tag.hash(&mut hasher);
            meta.last_modified.timestamp().hash(&mut hasher);
            hasher.finish()
        };
        Ok((bytes, (version_hash, meta.size)))
    }
}
