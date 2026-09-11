default: build

# ---------------------------------------------------------------- 构建

build:
	cargo build --release

install-bin: build
	cp target/release/svnui ~/.cargo/bin/svnui

# 一条命令装全套（二进制 + yazi 插件）
all: install-bin
	./target/release/svnui install-yazi --patch-init

# ---------------------------------------------------------------- 测试

# 离线单测：不依赖 svn、不依赖网络，CI 可跑
test:
	cargo test --offline

# 全量（含需要真 svn 的集成测试）
test-all:
	cargo test

# 快速跑一遍，不等 clippy 的慢检查
check:
	cargo check --all-targets

lint:
	cargo clippy --all-targets -- -D warnings

fmt:
	cargo fmt --all

# 提交前跑这个
pre: fmt lint test
	@echo "--- 三项铁律自检 ---"
	@! grep -r "std::process\|std::fs" src/domain/ && echo "✓ domain 零 IO" || echo "✗ domain 含 IO"
	@! grep -rn "sh -c\|cmd /C" src/ && echo "✓ 无 shell 注入" || echo "✗ 存在 shell 注入"

# ---------------------------------------------------------------- 样本

# 需要 svnadmin。用它重新生成 tests/fixtures/ 下的样本。
# 换 svn 版本后必须重跑一次，否则样本与真实输出脱节，解析会悄悄失效。
gen-fixtures:
	bash tests/fixtures/gen.sh

# ---------------------------------------------------------------- 基准

# 在你的大仓库里跑。首次 = 冷启动（跑 svn status + 写缓存）
# 二次 = 命中（只做树遍历）。这个差值决定了要不要上 daemon。
bench:
	@echo "--- 首次（冷）---"
	@time ./target/release/svnui status --porcelain > /dev/null
	@echo "--- 二次（热）---"
	@time ./target/release/svnui status --porcelain > /dev/null
	@echo "--- daemon 首次 ---"
	@./target/release/svnui daemon start
	@time ./target/release/svnui q --dir . > /dev/null
	@echo "--- daemon 二次（版本戳命中，应 <10ms）---"
	@time ./target/release/svnui q --dir . > /dev/null

# ---------------------------------------------------------------- 安装

# 装 yazi 插件
plugin:
	./target/release/svnui install-yazi

plugin-force:
	./target/release/svnui install-yazi --force --patch-init

# 看插件会装到哪（不写盘）
plugin-check:
	./target/release/svnui install-yazi --check

daemon-start:
	./target/release/svnui daemon start

daemon-stop:
	./target/release/svnui daemon stop

# ---------------------------------------------------------------- 清理

clean:
	cargo clean
	rm -rf tests/tmp

.PHONY: default build install-bin all test test-all check lint fmt pre \
        gen-fixtures bench plugin plugin-force plugin-check \
        daemon-start daemon-stop clean
