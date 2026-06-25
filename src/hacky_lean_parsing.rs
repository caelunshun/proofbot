//! Various hacky and completely incorrect parsing routines to analyze lean
//! source code. Should be replaced with a proper parser.

use std::range::Range;

/// Given a theorem name, returns the byte range containing
/// its `sorry` body, or `None` if the theorem isn't found
/// or doesn't have a `sorry` body.
pub fn query_theorem_body_byte_range(
    file_contents: &str,
    theorem_name: &str,
) -> Option<Range<usize>> {
    let theorem_start = query_theorem_start(file_contents, theorem_name)?;
    let sorry = file_contents[theorem_start..].find("sorry")? + theorem_start;
    let start_of_sorry_line = file_contents[..sorry].rfind("\n")? + 1;
    Some(Range {
        start: start_of_sorry_line,
        end: sorry + "sorry".len(),
    })
}

pub fn query_theorem_start(file_contents: &str, theorem_name: &str) -> Option<usize> {
    file_contents.find(format!("theorem {theorem_name}").as_str())
}
