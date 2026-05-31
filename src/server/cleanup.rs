//! Background cleanup task for expiring locks and auth cache entries.

use crate::auth::AuthCache;
use crate::webdav::LockStore;

type Locks = std::sync::Arc<tokio::sync::RwLock<LockStore>>;
type Cache = std::sync::Arc<std::sync::RwLock<AuthCache>>;
type Notify = std::sync::Arc<tokio::sync::Notify>;

/// Periodically prunes expired WebDAV locks and auth cache entries every 30 seconds.
/// Stops when notified via `shutdown`.
pub(super) async fn cleanup_task(locks: Locks, auth_cache: Cache, shutdown: Notify) {
    loop {
        tokio::select! {
            _ = tokio::time::sleep(std::time::Duration::from_secs(30)) => {
                let mut store = locks.write().await;
                let before = store.values().map(|v| v.len()).sum::<usize>();
                store.retain(|_path, infos| {
                    infos.retain(|l| !l.is_expired());
                    !infos.is_empty()
                });
                let after = store.values().map(|v| v.len()).sum::<usize>();
                if before > after {
                    tracing::debug!(
                        removed = before - after, remaining = after,
                        "cleanup expired locks"
                    );
                }

                let Ok(mut cache) = auth_cache.write() else {
                    tracing::warn!("auth cache lock poisoned during cleanup");
                    continue;
                };
                let before = cache.len();
                cache.retain(|_, expiry| *expiry > std::time::Instant::now());
                let after = cache.len();
                if before > after {
                    tracing::debug!(
                        removed = before - after, remaining = after,
                        "cleanup expired auth cache"
                    );
                }
            }
            _ = shutdown.notified() => {
                tracing::debug!("cleanup task shutting down");
                break;
            }
        }
    }
}
