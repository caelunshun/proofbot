use crate::{checker::LeanCheckResult, constrained_file_editor::ConstrainedFileEditor};
use cap_std::ambient_authority;
use rand::distr::{Alphanumeric, Distribution};
use std::{
    fs,
    io::ErrorKind,
    path::{Path, PathBuf},
};

#[derive(Debug)]
pub struct Context {
    dir: PathBuf,
    theorem_file_editor: ConstrainedFileEditor,
}

impl Context {
    pub fn new(project_dir: &Path, theorem_file: &str, theorem_name: &str) -> anyhow::Result<Self> {
        let theorem_file_editor =
            ConstrainedFileEditor::new(theorem_file, project_dir, theorem_name)?;
        Ok(Self {
            dir: project_dir.to_path_buf(),
            theorem_file_editor,
        })
    }

    pub fn theorem_file_editor_mut(&mut self) -> &mut ConstrainedFileEditor {
        &mut self.theorem_file_editor
    }

    pub fn check_scratch_file(&mut self, contents: &str) -> anyhow::Result<LeanCheckResult> {
        let scratch_name = format!(
            "scratch_{}.lean",
            (0..8)
                .map(|_| char::from(Alphanumeric.sample(&mut rand::rng())))
                .collect::<String>()
        );

        let path = self.dir.join(&scratch_name);
        if path.exists() {
            return self.check_scratch_file(contents);
        }

        fs::write(&path, contents.as_bytes())?;
        let result = crate::checker::check_lean_file(&scratch_name, &self.dir)?;
        fs::remove_file(&path)?;

        Ok(result)
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    pub fn read_file(&self, path: &str) -> anyhow::Result<Option<String>> {
        let dir = cap_std::fs::Dir::open_ambient_dir(&self.dir, ambient_authority())?;
        let contents = match dir.read_to_string(path.strip_prefix('/').unwrap_or(path)) {
            Ok(c) => c,
            Err(e) if e.kind() == ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(e.into()),
        };

        Ok(Some(render_with_line_numbers(&contents)))
    }
}

pub fn render_with_line_numbers(code: &str) -> String {
    let mut formatted_contents = String::new();
    for (i, line) in code.lines().enumerate() {
        formatted_contents.push_str(&format!("{}: {line}\n", i + 1));
    }
    formatted_contents
}

pub fn display_relative_path(path: &str) -> String {
    let mut path = path.to_owned();
    if !path.starts_with('/') {
        path.insert(0, '/');
    }

    path
}
