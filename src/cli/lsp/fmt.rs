use similar::{DiffOp, TextDiff};
use stylua_lib::{format_code, IndentType, LineEndings, OutputVerification};
use tower_lsp::lsp_types::{self, FormattingOptions, Position, Range, TextEdit, Url};

use crate::{config, opt};

use super::{position_to_offset, Backend};

fn get_config(
    backend: &Backend,
    uri: Url,
    FormattingOptions {
        tab_size,
        insert_spaces,
        properties: _,
        trim_trailing_whitespace: _,
        trim_final_newlines: _,
        insert_final_newline: _,
    }: FormattingOptions,
) -> stylua_lib::Config {
    let opts = opt::Opt {
        stdin_filepath: uri.to_file_path().ok(),
        check: true,
        output_format: opt::OutputFormat::Json,

        ..backend.opts.clone()
    };

    let mut config = config::ConfigResolver::new(&opts)
        .unwrap()
        .load_configuration(&uri.to_file_path().expect("uri is filepath"))
        .unwrap_or_default();

    config.indent_width = tab_size as usize;
    config.indent_type = if insert_spaces {
        IndentType::Spaces
    } else {
        IndentType::Tabs
    };

    config
}

pub fn diff_op_to_text_edit(
    diff_op: DiffOp,
    new_text: &str,
    line_endings: LineEndings,
) -> Option<TextEdit> {
    match diff_op {
        DiffOp::Equal { .. } => None,
        DiffOp::Delete {
            old_index,
            old_len,
            new_index: _,
        } => Some(TextEdit {
            range: Range {
                start: Position {
                    line: old_index as u32,
                    character: 0,
                },
                end: Position {
                    line: (old_index + old_len) as u32,
                    character: 0,
                },
            },
            new_text: "".to_string(),
        }),
        DiffOp::Insert {
            old_index,
            new_index,
            new_len,
        } => Some(TextEdit {
            range: Range {
                start: Position {
                    line: old_index as u32,
                    character: 0,
                },
                end: Position {
                    line: old_index as u32,
                    character: 0,
                },
            },
            new_text: new_text
                .lines()
                .skip(new_index)
                .take(new_len)
                .collect::<Vec<_>>()
                .join(match line_endings {
                    LineEndings::Unix => "\n",
                    LineEndings::Windows => "\r\n",
                }),
        }),
        DiffOp::Replace {
            old_index,
            old_len,
            new_index,
            new_len,
        } => Some(TextEdit {
            range: Range {
                start: Position {
                    line: old_index as u32,
                    character: 0,
                },
                end: Position {
                    line: (old_index + old_len) as u32,
                    character: 0,
                },
            },
            new_text: new_text
                .lines()
                .skip(new_index)
                .take(new_len)
                .collect::<Vec<_>>()
                .join(match line_endings {
                    LineEndings::Unix => "\n",
                    LineEndings::Windows => "\r\n",
                }),
        }),
    }
}

pub fn format_document(
    backend: &Backend,
    uri: Url,
    range: Option<lsp_types::Range>,
    format_options: FormattingOptions,
) -> Vec<TextEdit> {
    let document = backend
        .document_map
        .get(&uri)
        .expect("`textDocument/didOpen` must have been called before");

    let config = get_config(backend, uri, format_options);

    let range = range.map(|range| {
        let start = position_to_offset(range.start, &document);
        let end = position_to_offset(range.end, &document);
        stylua_lib::Range { start, end }
    });

    let result =
        format_code(&document, config, range, OutputVerification::None).expect("Can always format");

    TextDiff::from_lines(document.as_str(), result.as_str())
        .grouped_ops(0)
        .into_iter()
        .flat_map(|iter| {
            iter.into_iter()
                .filter_map(|diff_op| diff_op_to_text_edit(diff_op, &result, config.line_endings))
        })
        .collect::<Vec<_>>()
}
