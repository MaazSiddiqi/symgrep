#[derive(Debug)]
pub struct OutputRecord {
    pub path: String,
    pub line_num: u64,
    pub node_type: String,
    pub node_line_from: usize,
    pub node_line_to: usize,
    pub rendered_lines: String,
}

pub fn print_outputs(outputs: &[OutputRecord]) {
    let mut ordered = outputs.iter().collect::<Vec<_>>();
    ordered.sort_by(|a, b| {
        a.path
            .cmp(&b.path)
            .then(a.node_line_from.cmp(&b.node_line_from))
            .then(a.line_num.cmp(&b.line_num))
            .then(a.node_line_to.cmp(&b.node_line_to))
    });

    let mut last_path: Option<&str> = None;
    for out in ordered {
        if last_path != Some(out.path.as_str()) {
            if last_path.is_some() {
                println!();
            }
            println!(
                "{0}{1}{2}",
                crate::COLOR_PATH_DIM,
                out.path,
                crate::COLOR_RESET
            );
            last_path = Some(out.path.as_str());
        }

        println!(
            "  {0}{1}{2} {3}node_type={4} node_lines=[{5}..{6}]{2}\n{7}",
            crate::COLOR_LINE_NUM,
            out.line_num,
            crate::COLOR_RESET,
            crate::COLOR_META_MILD,
            out.node_type,
            out.node_line_from,
            out.node_line_to,
            out.rendered_lines
        );
    }
}
