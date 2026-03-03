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
    for out in outputs {
        println!(
            "{0}{1}{2}:{3}{4}{2} {5}node_type={6} node_lines=[{7}..{8}]{2}\n{9}",
            crate::COLOR_PATH_DIM,
            out.path,
            crate::COLOR_RESET,
            crate::COLOR_LINE_NUM,
            out.line_num,
            crate::COLOR_META_MILD,
            out.node_type,
            out.node_line_from,
            out.node_line_to,
            out.rendered_lines
        );
    }
}
