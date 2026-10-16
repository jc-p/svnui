//! 非阻塞合并文件事件；弹框或后台操作期间只记录变化，由 App 决定扫描时机。

use std::path::Path;
use std::sync::atomic::Ordering;
use std::time::Instant;

use crate::daemon::watcher::{self, MetaDirty, Pending};

pub(super) struct RefreshWatch {
    // 必须保留 watcher，释放它会停止监听。
    _watcher: notify::RecommendedWatcher,
    pending: Pending,
    metadata: MetaDirty,
    due: Option<Instant>,
}

impl RefreshWatch {
    pub(super) fn new(root: &Path) -> std::io::Result<Self> {
        let (watcher, pending, metadata) = watcher::watch(root)?;
        Ok(Self {
            _watcher: watcher,
            pending,
            metadata,
            due: None,
        })
    }

    pub(super) fn observe(&mut self, now: Instant) {
        let changed = {
            let mut paths = self.pending.lock().unwrap_or_else(|e| e.into_inner());
            let changed = !paths.is_empty();
            paths.clear();
            changed
        };
        if self.metadata.swap(false, Ordering::SeqCst) || changed {
            self.due = Some(now + watcher::DEBOUNCE);
        }
    }

    pub(super) fn take_due(&mut self, now: Instant) -> bool {
        if self.due.is_some_and(|due| now >= due) {
            self.due = None;
            true
        } else {
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn events_are_debounced_and_metadata_changes_are_not_lost() {
        let tmp = tempfile::tempdir().unwrap();
        let mut watch = RefreshWatch::new(tmp.path()).unwrap();
        let now = Instant::now();
        watch
            .pending
            .lock()
            .unwrap()
            .insert(tmp.path().join("a.txt"));
        watch.observe(now);
        assert!(!watch.take_due(now));
        watch.metadata.store(true, Ordering::SeqCst);
        watch.observe(now + watcher::DEBOUNCE);
        assert!(!watch.take_due(now + watcher::DEBOUNCE));
        assert!(watch.take_due(now + watcher::DEBOUNCE * 2));
        assert!(!watch.take_due(now + watcher::DEBOUNCE * 3));
    }
}
