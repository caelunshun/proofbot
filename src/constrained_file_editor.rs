use crate::{checker::LeanCheckResult, hacky_lean_parsing::query_theorem_start};
use anyhow::bail;
use std::{
    fs,
    path::{Path, PathBuf},
};

#[derive(Debug)]
pub struct ConstrainedFileEditor {
    file_path: PathBuf,
    relative_path: String,
    original_contents: String,
    edits: Vec<Edit>,
    project_dir: PathBuf,
    theorem_name: String,
}

impl ConstrainedFileEditor {
    pub fn new(
        relative_path: &str,
        project_dir: &Path,
        theorem_name: &str,
    ) -> anyhow::Result<Self> {
        let file_path = project_dir.join(relative_path);
        let original_contents = fs::read_to_string(&file_path)?;

        if crate::hacky_lean_parsing::query_theorem_body_byte_range(
            &original_contents,
            theorem_name,
        )
        .is_none()
        {
            bail!("could not find specified theorem");
        }

        Ok(Self {
            file_path,
            relative_path: relative_path.to_owned(),
            original_contents,
            edits: Vec::new(),
            project_dir: project_dir.to_path_buf(),
            theorem_name: theorem_name.to_owned(),
        })
    }

    pub fn has_theorem_proof(&self) -> bool {
        self.edits
            .iter()
            .any(|edit| matches!(edit, Edit::ReplaceTheoremSorry(_)))
    }

    pub fn update_theorem_proof(&mut self, proof: &str) -> anyhow::Result<LeanCheckResult> {
        self.apply_edit_if_valid(Edit::ReplaceTheoremSorry(proof.to_owned()))
    }

    pub fn update_lemma(
        &mut self,
        lemma_name: &str,
        lemma_code: &str,
    ) -> anyhow::Result<LeanCheckResult> {
        self.apply_edit_if_valid(Edit::InsertLemma {
            lemma_name: lemma_name.to_owned(),
            lemma_contents: lemma_code.to_owned(),
        })
    }

    pub fn add_import(&mut self, import: &str) -> anyhow::Result<LeanCheckResult> {
        self.apply_edit_if_valid(Edit::AddImport(import.to_owned()))
    }

    fn apply_edit_if_valid(&mut self, edit: Edit) -> anyhow::Result<LeanCheckResult> {
        let mut new_edits = self.edits.clone();
        unify_edits(&mut new_edits, edit);

        let contents = apply_edits(&self.original_contents, &new_edits, &self.theorem_name);
        fs::write(&self.file_path, contents.as_bytes())?;

        let result = crate::checker::check_lean_file(&self.relative_path, &self.project_dir)?;
        if matches!(result, LeanCheckResult::Success) {
            self.edits = new_edits;
        } else {
            // Revert changes
            let contents = apply_edits(&self.original_contents, &self.edits, &self.theorem_name);
            fs::write(&self.file_path, contents.as_bytes())?;
        }

        Ok(result)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Edit {
    ReplaceTheoremSorry(String),
    InsertLemma {
        lemma_name: String,
        lemma_contents: String,
    },
    AddImport(String),
}

fn unify_edits(existing_edits: &mut Vec<Edit>, new_edit: Edit) {
    match &new_edit {
        Edit::ReplaceTheoremSorry(_) => {
            existing_edits.retain(|edit| !matches!(edit, Edit::ReplaceTheoremSorry(_)));
            existing_edits.insert(0, new_edit);
        }
        Edit::InsertLemma {
            lemma_name: new_name,
            lemma_contents: new_body,
        } => {
            if new_body.is_empty() {
                // Removal of lemma
                existing_edits.retain(|existing_edit| !matches!(existing_edit, Edit::InsertLemma { lemma_name, .. } if lemma_name == new_name));
            } else {
                for existing_edit in existing_edits.iter_mut() {
                    if let Edit::InsertLemma {
                        lemma_name,
                        lemma_contents: lemma_body,
                    } = existing_edit
                        && lemma_name == new_name
                    {
                        *lemma_body = new_body.clone();
                        return;
                    }
                }
                existing_edits.push(new_edit);
            }
        }
        Edit::AddImport(_) => {
            if !existing_edits.contains(&new_edit) {
                existing_edits.push(new_edit);
            }
        }
    }
}

fn apply_edits(original_contents: &str, edits: &[Edit], theorem_name: &str) -> String {
    let mut contents = original_contents.to_owned();
    for edit in edits {
        match edit {
            Edit::ReplaceTheoremSorry(new_body) => {
                let range = crate::hacky_lean_parsing::query_theorem_body_byte_range(
                    &contents,
                    theorem_name,
                )
                .expect("oops");
                contents.replace_range(range, new_body);
            }
            Edit::InsertLemma { lemma_contents, .. } => {
                let theorem_start = query_theorem_start(&contents, theorem_name).expect("oops");
                contents.insert_str(theorem_start, &format!("{lemma_contents}\n\n"));
            }
            Edit::AddImport(import) => {
                contents.insert_str(0, &format!("import {import}"));
            }
        }
    }
    contents
}
