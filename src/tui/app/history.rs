//! 历史记录的后台加载；状态与结果路由仍由 App 持有。

use super::{App, BgDone, Mode};

impl App {
    /// 首次打开历史时加载多少条。
    ///
    /// 100 而不是 200：svn log 是**联网**操作，每多一条都要等服务器。
    /// 首屏 100 条足够翻找，更早的靠搜索（服务端 `--search`）定位。
    /// 每次加载多少条提交。来自配置 `log.page_size`（默认 100）。
    ///
    /// `svn log` 是联网操作：拉太多要等服务器，拉太少要反复翻页。
    /// 上下限卡在 10..=2000 —— 配置写错也不至于一次拉十万条。
    pub(super) fn log_page(&self) -> usize {
        self.cfg.log.page_size.clamp(10, 2000)
    }

    pub(super) fn open_log(&mut self) {
        self.spawn_log_load(None, self.log_page(), false);
        self.mode = Mode::Log;
    }

    /// 拉日志。`search` 非空时走服务端 `--search`。
    ///
    /// `append`：是否把结果追加到现有列表（搜索是替换，不是追加）。
    pub(super) fn spawn_log_load(&mut self, search: Option<String>, limit: usize, append: bool) {
        let svn = self.svn.clone();
        let tx = self.tx.clone();
        // 版本不够就退回本地过滤：旧 svn 不认识 --search，
        // 直接传会报错。这种情况下至少还能搜已加载的 100 条。
        let use_search = search.is_some() && svn.supports_log_search();
        self.busy = Some(match (&search, use_search) {
            (Some(kw), true) => format!("搜索“{}”…", kw),
            (Some(_), false) => "搜索（本地）…".into(),
            (None, _) => "读取历史".into(),
        });
        std::thread::spawn(move || {
            let r = if use_search {
                svn.log_search(limit, &[], search.as_deref())
            } else {
                svn.log(limit, &[])
            };
            let _ = tx.send(BgDone::LogLoad(r, search.clone(), append));
        });
    }

    /// 查看某个历史版本改了什么（`svn diff -c REV`）。
    ///
    /// 用在日志面板里按 Enter：右侧已经能看到"改了哪些文件"（`-v` 摘要），
    /// 但要看**具体改了什么**得拉完整 diff。
    pub(super) fn open_rev_diff(&mut self, rev: u64) {
        let svn = self.svn.clone();
        let tx = self.tx.clone();
        self.busy = Some(format!("读取 r{} 的改动", rev));
        std::thread::spawn(move || {
            let r = svn.rev_diff(rev).map(|o| o.stdout);
            let _ = tx.send(BgDone::RevDiff(rev, r));
        });
    }

    pub(super) fn spawn_log_detail(&mut self) {
        let Some(rev) = self.logpanel.as_ref().and_then(|p| p.need_detail()) else {
            return;
        };
        let svn = self.svn.clone();
        let tx = self.tx.clone();
        std::thread::spawn(move || {
            let lines = match svn.log_rev(&rev.to_string(), true) {
                Ok(es) => es
                    .into_iter()
                    .flat_map(|e| {
                        e.paths
                            .iter()
                            .map(|p| format!(" {} {}", p.action, p.path))
                            .collect::<Vec<_>>()
                    })
                    .collect::<Vec<_>>(),
                Err(e) => vec![format!("读取失败：{}", e)],
            };
            let _ = tx.send(BgDone::LogDetail(rev, lines));
        });
    }
}
