//! Filesystem abstraction: writes pages to disk, S3 (later), or in-memory.
//!
//! Two impls ship in v0:
//! - `LocalFilesystem`: writes under a root directory.
//! - `InMemoryFilesystem`: HashMap-backed; for tests and harness-internal use.
//!
//! All paths are relative to a configured root and use `/` separators. The
//! filesystem normalizes to OS-native separators only at the boundary.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use async_trait::async_trait;

use crate::error::RenderError;

#[async_trait]
pub trait Filesystem: Send + Sync {
    async fn write(&self, path: &str, bytes: &[u8]) -> Result<(), RenderError>;
    async fn read(&self, path: &str) -> Result<Option<Vec<u8>>, RenderError>;
    async fn list(&self, prefix: &str) -> Result<Vec<String>, RenderError>;
    async fn remove(&self, path: &str) -> Result<(), RenderError>;
}

pub struct LocalFilesystem {
    root: PathBuf,
}

impl LocalFilesystem {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self {
            root: root.into(),
        }
    }

    fn resolve(&self, path: &str) -> PathBuf {
        // path is "/" separated and relative; reject "..".
        let mut out = self.root.clone();
        for segment in path.split('/').filter(|s| !s.is_empty() && *s != ".") {
            if segment == ".." {
                continue;
            }
            out.push(segment);
        }
        out
    }
}

#[async_trait]
impl Filesystem for LocalFilesystem {
    async fn write(&self, path: &str, bytes: &[u8]) -> Result<(), RenderError> {
        let full = self.resolve(path);
        if let Some(parent) = full.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }
        tokio::fs::write(full, bytes).await?;
        Ok(())
    }

    async fn read(&self, path: &str) -> Result<Option<Vec<u8>>, RenderError> {
        let full = self.resolve(path);
        match tokio::fs::read(&full).await {
            Ok(bytes) => Ok(Some(bytes)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    async fn list(&self, prefix: &str) -> Result<Vec<String>, RenderError> {
        let dir = self.resolve(prefix);
        if !dir.exists() {
            return Ok(Vec::new());
        }
        let mut out = Vec::new();
        walk(&self.root, &dir, &mut out)?;
        out.sort();
        Ok(out)
    }

    async fn remove(&self, path: &str) -> Result<(), RenderError> {
        let full = self.resolve(path);
        match tokio::fs::remove_file(&full).await {
            Ok(_) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(e.into()),
        }
    }
}

fn walk(root: &Path, dir: &Path, out: &mut Vec<String>) -> Result<(), RenderError> {
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() {
            walk(root, &path, out)?;
        } else if let Ok(rel) = path.strip_prefix(root) {
            // Convert to forward-slash representation.
            let s = rel
                .components()
                .map(|c| c.as_os_str().to_string_lossy().into_owned())
                .collect::<Vec<_>>()
                .join("/");
            out.push(s);
        }
    }
    Ok(())
}

#[derive(Default)]
pub struct InMemoryFilesystem {
    files: Mutex<BTreeMap<String, Vec<u8>>>,
}

impl InMemoryFilesystem {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn snapshot(&self) -> BTreeMap<String, Vec<u8>> {
        self.files.lock().unwrap().clone()
    }
}

#[async_trait]
impl Filesystem for InMemoryFilesystem {
    async fn write(&self, path: &str, bytes: &[u8]) -> Result<(), RenderError> {
        self.files
            .lock()
            .unwrap()
            .insert(path.trim_start_matches('/').to_string(), bytes.to_vec());
        Ok(())
    }

    async fn read(&self, path: &str) -> Result<Option<Vec<u8>>, RenderError> {
        Ok(self
            .files
            .lock()
            .unwrap()
            .get(path.trim_start_matches('/'))
            .cloned())
    }

    async fn list(&self, prefix: &str) -> Result<Vec<String>, RenderError> {
        let p = prefix.trim_start_matches('/');
        let mut out: Vec<String> = self
            .files
            .lock()
            .unwrap()
            .keys()
            .filter(|k| p.is_empty() || k.starts_with(p))
            .cloned()
            .collect();
        out.sort();
        Ok(out)
    }

    async fn remove(&self, path: &str) -> Result<(), RenderError> {
        self.files
            .lock()
            .unwrap()
            .remove(path.trim_start_matches('/'));
        Ok(())
    }
}
