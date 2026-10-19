//! 跟随选中文件的预览调度：一个运行任务、一个可覆盖待办和一个结果槽。

use std::io;
use std::sync::{Arc, Condvar, Mutex};
use std::thread;

use crate::svn::command::CancelToken;

use super::panels::preview::PreviewPanel;

type LoadPreview = Box<dyn FnOnce(&CancelToken) -> Option<PreviewPanel> + Send>;

struct Job {
    generation: u64,
    cancel: CancelToken,
    load: LoadPreview,
}

#[derive(Default)]
struct State {
    pending: Option<Job>,
    running: Option<CancelToken>,
    completed: Option<(u64, PreviewPanel)>,
    stopped: bool,
}

/// 不用于写操作；取消标记只交给只读预览的 SVN 命令。
pub(super) struct PreviewWorker {
    shared: Arc<(Mutex<State>, Condvar)>,
}

impl PreviewWorker {
    pub(super) fn new() -> io::Result<Self> {
        let shared = Arc::new((Mutex::new(State::default()), Condvar::new()));
        let worker_shared = Arc::clone(&shared);
        thread::Builder::new()
            .name("svnui-preview".into())
            .spawn(move || Self::run(worker_shared))?;
        Ok(Self { shared })
    }

    pub(super) fn submit(
        &self,
        generation: u64,
        load: impl FnOnce(&CancelToken) -> Option<PreviewPanel> + Send + 'static,
    ) {
        let (lock, wake) = &*self.shared;
        let mut state = lock.lock().unwrap_or_else(|e| e.into_inner());
        Self::cancel_locked(&mut state);
        if !state.stopped {
            state.pending = Some(Job {
                generation,
                cancel: CancelToken::default(),
                load: Box::new(load),
            });
            wake.notify_one();
        }
    }

    pub(super) fn cancel(&self) {
        let mut state = self.shared.0.lock().unwrap_or_else(|e| e.into_inner());
        Self::cancel_locked(&mut state);
    }

    fn cancel_locked(state: &mut State) {
        if let Some(running) = &state.running {
            running.cancel();
        }
        state.pending = None;
        state.completed = None;
    }

    pub(super) fn take_result(&self) -> Option<(u64, PreviewPanel)> {
        self.shared
            .0
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .completed
            .take()
    }

    fn run(shared: Arc<(Mutex<State>, Condvar)>) {
        let (lock, wake) = &*shared;
        loop {
            let job = {
                let mut state = lock.lock().unwrap_or_else(|e| e.into_inner());
                while !state.stopped && state.pending.is_none() {
                    state = wake.wait(state).unwrap_or_else(|e| e.into_inner());
                }
                if state.stopped {
                    return;
                }
                let job = state.pending.take().expect("pending preview checked above");
                state.running = Some(job.cancel.clone());
                job
            };
            // 文件解码可能无法中断；锁不跨过解析/命令，前后均检查取消。
            let panel = if job.cancel.is_cancelled() {
                None
            } else {
                // 单个解析器 panic 只影响本次预览，保持线程能处理下一次选择。
                match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    (job.load)(&job.cancel)
                })) {
                    Ok(panel) => panel,
                    Err(_) => Some(PreviewPanel::notice(
                        "预览失败",
                        "内容解析发生异常；可以选择其他文件，或返回文件树按 o 使用系统应用打开",
                    )),
                }
            };
            let mut state = lock.lock().unwrap_or_else(|e| e.into_inner());
            state.running = None;
            if !state.stopped && !job.cancel.is_cancelled() {
                state.completed = panel.map(|panel| (job.generation, panel));
            }
        }
    }
}

impl Drop for PreviewWorker {
    fn drop(&mut self) {
        let (lock, wake) = &*self.shared;
        let mut state = lock.lock().unwrap_or_else(|e| e.into_inner());
        state.stopped = true;
        Self::cancel_locked(&mut state);
        // 不 join 正在解码的线程；它完成后观察 stopped 并自行退出。
        wake.notify_one();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;
    use std::time::{Duration, Instant};

    fn wait_result(worker: &PreviewWorker) -> (u64, PreviewPanel) {
        let deadline = Instant::now() + Duration::from_secs(3);
        loop {
            if let Some(result) = worker.take_result() {
                return result;
            }
            assert!(Instant::now() < deadline, "preview result timed out");
            thread::sleep(Duration::from_millis(5));
        }
    }

    #[test]
    fn latest_pending_replaces_middle_request_and_cancels_running() {
        let worker = PreviewWorker::new().unwrap();
        let (started_tx, started_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        let (ran_tx, ran_rx) = mpsc::channel();
        worker.submit(1, move |cancel| {
            started_tx.send(cancel.clone()).unwrap();
            release_rx.recv_timeout(Duration::from_secs(3)).unwrap();
            Some(PreviewPanel::notice("old", "must not win"))
        });
        let token = started_rx.recv_timeout(Duration::from_secs(3)).unwrap();
        let middle_tx = ran_tx.clone();
        worker.submit(2, move |_| {
            middle_tx.send(2).unwrap();
            Some(PreviewPanel::notice("middle", "must not run"))
        });
        worker.submit(3, move |_| {
            ran_tx.send(3).unwrap();
            Some(PreviewPanel::notice("latest", "latest"))
        });
        assert!(token.is_cancelled());
        assert_eq!(
            worker
                .shared
                .0
                .lock()
                .unwrap()
                .pending
                .as_ref()
                .unwrap()
                .generation,
            3
        );
        assert!(
            ran_rx.try_recv().is_err(),
            "no second job may run concurrently"
        );
        release_tx.send(()).unwrap();
        assert_eq!(wait_result(&worker).0, 3);
        assert_eq!(ran_rx.recv_timeout(Duration::from_secs(3)).unwrap(), 3);
        assert!(ran_rx.try_recv().is_err());
    }

    #[test]
    fn cancel_clears_pending_and_discards_late_completion() {
        let worker = PreviewWorker::new().unwrap();
        let (started_tx, started_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        worker.submit(1, move |cancel| {
            started_tx.send(cancel.clone()).unwrap();
            release_rx.recv_timeout(Duration::from_secs(3)).unwrap();
            Some(PreviewPanel::notice("old", "old"))
        });
        let token = started_rx.recv_timeout(Duration::from_secs(3)).unwrap();
        worker.submit(2, |_| panic!("cancel must remove pending request"));
        worker.cancel();
        assert!(token.is_cancelled());
        release_tx.send(()).unwrap();
        let deadline = Instant::now() + Duration::from_secs(3);
        while worker.shared.0.lock().unwrap().running.is_some() {
            assert!(Instant::now() < deadline);
            thread::sleep(Duration::from_millis(5));
        }
        assert!(worker.shared.0.lock().unwrap().pending.is_none());
        assert!(worker.take_result().is_none());
    }

    #[test]
    fn drop_does_not_wait_for_an_uninterruptible_loader() {
        let worker = PreviewWorker::new().unwrap();
        let (started_tx, started_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        worker.submit(1, move |cancel| {
            started_tx.send(cancel.clone()).unwrap();
            let _ = release_rx.recv_timeout(Duration::from_secs(3));
            None
        });
        let token = started_rx.recv_timeout(Duration::from_secs(3)).unwrap();
        let start = Instant::now();
        drop(worker);
        assert!(start.elapsed() < Duration::from_secs(1));
        assert!(token.is_cancelled());
        release_tx.send(()).unwrap();
    }

    #[test]
    fn panicking_loader_returns_notice_and_next_request_still_completes() {
        let worker = PreviewWorker::new().unwrap();
        worker.submit(1, |_| panic!("injected preview parser panic"));
        let (generation, panel) = wait_result(&worker);
        assert_eq!(generation, 1);
        assert_eq!(panel.kind(), super::super::panels::preview::Kind::Notice);
        assert!(worker.shared.0.lock().unwrap().running.is_none());
        worker.submit(2, |_| Some(PreviewPanel::file("next", "next content")));
        let (generation, panel) = wait_result(&worker);
        assert_eq!(generation, 2);
        assert_eq!(panel.kind(), super::super::panels::preview::Kind::File);
        assert!(worker.shared.0.lock().unwrap().running.is_none());
    }
}
