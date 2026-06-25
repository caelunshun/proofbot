use include_dir::{Dir, DirEntry, include_dir};
use minijinja::Environment;
use serde::Serialize;

pub struct TemplateEngine {
    env: Environment<'static>,
}

impl TemplateEngine {
    pub fn load() -> Self {
        let mut env = Environment::empty();

        static TEMPLATE_DIR: Dir = include_dir!("$CARGO_MANIFEST_DIR/prompts");

        fn visit(entry: &'static DirEntry, env: &mut Environment) {
            if let Some(dir) = entry.as_dir() {
                for child in dir.entries() {
                    visit(child, env);
                }
            } else if let Some(file) = entry.as_file() {
                env.add_template(file.path().to_str().unwrap(), file.contents_utf8().unwrap())
                    .expect("failed to compile template");
            }
        }

        for entry in TEMPLATE_DIR.entries() {
            visit(entry, &mut env);
        }

        Self { env }
    }

    pub fn render(&self, template_path: &str, data: impl Serialize) -> String {
        self.env
            .get_template(template_path)
            .unwrap()
            .render(minijinja::Value::from_serialize(data))
            .unwrap()
    }
}

pub mod system_prompt {
    use serde::Serialize;

    #[derive(Debug, Serialize)]
    pub struct Data {
        pub project_file_tree: String,
    }
}

pub mod user_prompt {
    use serde::Serialize;

    #[derive(Debug, Serialize)]
    pub struct Data {
        pub theorem_file_path: String,
        pub theorem_file_contents: String,
        pub theorem_name: String,
    }
}

pub mod add_import {
    use serde::Serialize;

    #[derive(Debug, Serialize)]
    pub struct SuccessData {}

    #[derive(Debug, Serialize)]
    pub struct FailureData {
        pub failure_reason: String,
    }
}

pub mod read_file {
    use serde::Serialize;

    #[derive(Debug, Serialize)]
    pub struct SuccessData {
        pub file_path: String,
        pub file_contents: String,
    }

    #[derive(Debug, Serialize)]
    pub struct FailureData {
        pub file_path: String,
    }
}

pub mod recall_directory_structure {
    use serde::Serialize;

    #[derive(Debug, Serialize)]
    pub struct SuccessData {
        pub project_file_tree: String,
    }
}

pub mod update_lemma {
    use serde::Serialize;

    #[derive(Debug, Serialize)]
    pub struct SuccessData {
        pub lemma_name: String,
    }

    #[derive(Debug, Serialize)]
    pub struct FailureData {
        pub lemma_name: String,
        pub failure_reason: String,
        pub failed_file_contents: String,
    }
}

pub mod update_proof {
    use serde::Serialize;

    #[derive(Debug, Serialize)]
    pub struct SuccessData {}

    #[derive(Debug, Serialize)]
    pub struct FailureData {
        pub failure_reason: String,
        pub failed_file_contents: String,
    }
}

pub mod write_scratch_file {
    use serde::Serialize;

    #[derive(Debug, Serialize)]
    pub struct SuccessData {
        pub lean_output: String,
    }
}
