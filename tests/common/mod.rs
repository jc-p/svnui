#![allow(dead_code)]

use std::path::PathBuf;

/// 读取 `tests/fixtures/` 下的样本。
pub fn fixture(name: &str) -> String {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests").join("fixtures").join(name);
    std::fs::read_to_string(&p).unwrap_or_else(|e| panic!("读取 fixture `{name}` 失败: {e}"))
}
