//! Byte storage for uploaded documents. Filesystem-backed; a MinIO/S3 impl
//! slots in behind the same three functions.

use crate::error::{ApiError, ApiResult};
use std::path::{Path, PathBuf};
use tokio::fs;

fn path_for(root: &str, key: &str) -> ApiResult<PathBuf> {
    let root = Path::new(root);
    let full = root.join(key);
    // Reject keys that escape the root.
    if !full.starts_with(root) || key.contains("..") {
        return Err(ApiError::validation("Invalid storage key."));
    }
    Ok(full)
}

pub async fn put(root: &str, key: &str, bytes: &[u8]) -> ApiResult<()> {
    let path = path_for(root, key)?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .await
            .map_err(|_| ApiError::server_error())?;
    }
    fs::write(&path, bytes)
        .await
        .map_err(|_| ApiError::server_error())?;
    Ok(())
}

pub async fn delete(root: &str, key: &str) -> ApiResult<()> {
    let path = path_for(root, key)?;
    let _ = fs::remove_file(&path).await;
    Ok(())
}
