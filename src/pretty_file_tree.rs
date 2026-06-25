use anyhow::Context;
use ptree::{TreeBuilder, write_tree};
use std::{fs, path::Path};

pub fn generate_file_tree(
    dir: &Path,
    filename_filter: impl Fn(&str) -> bool,
) -> anyhow::Result<String> {
    let mut tree_builder = TreeBuilder::new("/".into());

    fn visit(
        tree_builder: &mut TreeBuilder,
        entry: &Path,
        filename_filter: &impl Fn(&str) -> bool,
    ) -> anyhow::Result<()> {
        let name = entry
            .file_name()
            .context("no file name")?
            .to_str()
            .context("not valid utf8")?;

        if name.starts_with('.') {
            return Ok(());
        }

        if entry.is_dir() {
            tree_builder.begin_child(name.to_owned());
            for child in fs::read_dir(entry)? {
                visit(tree_builder, &child?.path(), filename_filter)?;
            }
            tree_builder.end_child();
        } else if filename_filter(name) {
            tree_builder.add_empty_child(name.to_owned());
        }

        Ok(())
    }

    for entry in fs::read_dir(dir)? {
        visit(&mut tree_builder, &entry?.path(), &filename_filter)?;
    }

    let mut s = Vec::<u8>::new();
    write_tree(&tree_builder.build(), &mut s)?;
    Ok(String::from_utf8(s)?)
}
