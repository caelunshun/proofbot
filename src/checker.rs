use std::{fs, path::Path, process::Command};

pub fn check_lean_file(
    relative_path: &str,
    project_root: &Path,
) -> anyhow::Result<LeanCheckResult> {
    let output = Command::new("lake")
        .args(["lean", relative_path])
        .current_dir(project_root)
        .output()?;

    Ok(if output.status.success() {
        LeanCheckResult::Success
    } else {
        let output = String::from_utf8(output.stdout)?;
        tracing::info!("Lean check failure: {output}");
        let source_code = fs::read_to_string(project_root.join(relative_path))?;
        LeanCheckResult::Failure {
            source_code,
            output,
        }
    })
}

#[derive(Debug, Clone)]
pub enum LeanCheckResult {
    Success,
    Failure { source_code: String, output: String },
}
