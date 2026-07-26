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
}

impl LocalFile {
    /// # Errors
    ///
    /// [`SourceError::NotFound`] when the path does not exist;
    /// [`SourceError::Io`] for any other open/stat failure.
    pub fn open(path: &Path) -> Result<Self, SourceError> {
        let file = File::open(path)?;
        let len = file.metadata().map_err(SourceError::from)?.len();
        Ok(Self {
            file: Arc::new(file),
            len,
        })
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
