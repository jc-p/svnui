use serde::Serialize;
use serde_json::json;

/// 成功信封：`{"ok":true,"data":...}`。
pub fn ok<T: Serialize>(data: &T) -> String {
    serde_json::to_string(&json!({ "ok": true, "data": data }))
        .unwrap_or_else(|e| err("serialize", &e.to_string()))
}

/// 失败信封：`{"ok":false,"error":{"kind":..,"message":..}}`。
///
/// `kind` 用稳定字符串（调用方据此决定提示方式），不直接暴露 Rust 内部结构。
pub fn err(kind: &str, message: &str) -> String {
    serde_json::to_string(&json!({
        "ok": false,
        "error": { "kind": kind, "message": message }
    }))
    .unwrap_or_else(|_| r#"{"ok":false,"error":{"kind":"internal","message":"serialize failed"}}"#.to_string())
}

/// 领域错误 → kind 字符串。
pub fn kind_of(e: &crate::domain::Error) -> &'static str {
    use crate::domain::Error;
    match e {
        Error::SvnNotFound => "svn-not-found",
        Error::NotWorkingCopy(_) => "not-working-copy",
        Error::Locked(_) => "locked",
        Error::Timeout(_) => "timeout",
        Error::Cancelled => "cancelled",
        Error::SvnFailed { .. } => "svn-failed",
        // 带人话解释的 svn 失败：对外仍归类为 svn-failed，
        // 只是 summary/detail 已经是翻译过的。
        Error::Explained { .. } => "svn-failed",
        Error::Parse(_) => "parse",
        Error::Io(_) => "io",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::Error;

    #[test]
    fn ok_envelope_is_parseable() {
        let s = ok(&vec!["a", "b"]);
        let v: serde_json::Value = serde_json::from_str(&s).unwrap();
        assert_eq!(v["ok"], true);
        assert_eq!(v["data"][0], "a");
    }

    #[test]
    fn err_envelope_has_kind_and_message() {
        let s = err("locked", "工作副本被锁");
        let v: serde_json::Value = serde_json::from_str(&s).unwrap();
        assert_eq!(v["ok"], false);
        assert_eq!(v["error"]["kind"], "locked");
        assert_eq!(v["error"]["message"], "工作副本被锁");
    }

    #[test]
    fn kind_is_stable_string() {
        assert_eq!(kind_of(&Error::SvnNotFound), "svn-not-found");
        assert_eq!(kind_of(&Error::Locked("/x".into())), "locked");
    }
}
