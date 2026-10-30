# README 界面配图

这三张 PNG 来自 `examples/docs_capture.rs` 创建的本地 `file://` SVN 示例工作副本，由实际 `App` 经 ratatui `TestBackend` 渲染，再转换为图片。文件、作者及提交说明仅作演示，不包含真实项目。它们是界面渲染图，不是对真实远端仓库的操作记录。

需要本机有 Rust、SVN（含 `svnadmin`）、`rsvg-convert` 和支持中文的等宽字体。生成器使用 Maple Mono NF CN；缺失时由系统回退，字体效果可能不同。

在仓库根目录执行，SVG 中间产物放在临时目录：

```bash
cargo run --offline --locked --example docs_capture -- /tmp/svnui-docs-images
rsvg-convert /tmp/svnui-docs-images/overview.svg -o docs/images/overview.png
rsvg-convert /tmp/svnui-docs-images/commit.svg -o docs/images/commit.png
rsvg-convert /tmp/svnui-docs-images/help.svg -o docs/images/help.png
```

预期得到三张 1312 × 780 的图片：主面板差异预览、提交框、分类快捷键帮助。生成器检查预期画面内容，临时工作副本与隔离配置在结束后自动清理；不会运行模型、提交远端或改写用户配置。完成后检查图片中文字、边框与内容，删除 `/tmp/svnui-docs-images` 即可。
