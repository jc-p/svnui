//! 分支命令共用确认流程；干跑只读，取消确认后立即返回。

use crate::domain::{Error, Result};
use crate::policy::{danger_of, judge, Verdict};
use crate::svn::workflow::{WorkflowOperation, WorkflowRequest};
use crate::svn::Svn;

pub fn run(
    svn: &Svn,
    request: WorkflowRequest,
    dry_run: bool,
    yes: bool,
    tty: bool,
    json: bool,
) -> Result<String> {
    let plan = svn.prepare_workflow(&request)?;
    let op = plan.operation().policy_op();
    let description = plan.describe().join("\n");
    let verdict = judge(op, danger_of(op), dry_run, yes, tty);
    let output = match verdict {
        Verdict::DryRun => format!(
            "[干跑] {}\n{description}\n未实际执行。",
            plan.operation().title()
        ),
        Verdict::Refused { reason } => return Err(Error::Parse(reason)),
        Verdict::NeedConfirm { .. } => {
            let prompt = format!("{}\n{description}\n确认执行？", plan.operation().title());
            if !crate::policy::confirm(&prompt) {
                return Ok(if json {
                    crate::output::ok(&serde_json::json!({"cancelled": true, "plan": plan}))
                } else {
                    "已取消".into()
                });
            }
            plan.execute(svn)?.stdout
        }
        Verdict::Proceed => plan.execute(svn)?.stdout,
    };
    let preview = request.operation == WorkflowOperation::MergePreview;
    if json {
        Ok(crate::output::ok(&serde_json::json!({
            "plan": plan,
            "dry_run": dry_run || preview,
            "output": output,
        })))
    } else if preview {
        Ok(format!(
            "合并预演（未应用改动、未提交）\n{description}\n{output}"
        ))
    } else {
        Ok(output)
    }
}
