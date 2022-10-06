extern crate micromark;
extern crate swc_common;
extern crate swc_ecma_ast;
extern crate swc_ecma_parser;
use micromark::{MdxExpressionKind, MdxSignal};
use swc_common::{
    source_map::Pos, sync::Lrc, BytePos, FileName, FilePathMapping, SourceFile, SourceMap, Spanned,
};
use swc_ecma_ast::{EsVersion, Expr, Module};
use swc_ecma_codegen::{text_writer::JsWriter, Emitter};
use swc_ecma_parser::{
    error::Error as SwcError, parse_file_as_expr, parse_file_as_module, EsConfig, Syntax,
};

// To do:
// Use lexer in the future:
// <https://docs.rs/swc_ecma_parser/0.99.1/swc_ecma_parser/lexer/index.html>

/// Lex ESM in MDX with SWC.
#[allow(dead_code)]
pub fn parse_esm(value: &str) -> MdxSignal {
    let (file, syntax, version) = create_config(value.to_string());
    let mut errors = vec![];
    let result = parse_file_as_module(&file, syntax, version, None, &mut errors);

    match result {
        Err(error) => swc_error_to_signal(&error, value.len(), 0, "esm"),
        Ok(tree) => {
            if errors.is_empty() {
                check_esm_ast(&tree)
            } else {
                if errors.len() > 1 {
                    println!("parse_esm: todo: multiple errors? {:?}", errors);
                }
                swc_error_to_signal(&errors[0], value.len(), 0, "esm")
            }
        }
    }
}

/// Parse ESM in MDX with SWC.
/// To do: figure out how to fix positional info.
/// See `drop_span` in `swc_ecma_utils` for inspiration?
#[allow(dead_code)]
pub fn parse_esm_to_tree(value: &str) -> Result<swc_ecma_ast::Module, String> {
    let (file, syntax, version) = create_config(value.to_string());
    let mut errors = vec![];

    let result = parse_file_as_module(&file, syntax, version, None, &mut errors);

    match result {
        Err(error) => Err(swc_error_to_string(&error)),
        Ok(module) => {
            if errors.is_empty() {
                Ok(module)
            } else {
                if errors.len() > 1 {
                    println!("parse_esm_to_tree: todo: multiple errors? {:?}", errors);
                }
                Err(swc_error_to_string(&errors[0]))
            }
        }
    }
}

/// Lex expressions in MDX with SWC.
#[allow(dead_code)]
pub fn parse_expression(value: &str, kind: &MdxExpressionKind) -> MdxSignal {
    // Empty expressions are OK.
    if matches!(kind, MdxExpressionKind::Expression)
        && matches!(whitespace_and_comments(0, value), MdxSignal::Ok)
    {
        return MdxSignal::Ok;
    }

    // For attribute expression, a spread is needed, for which we have to prefix
    // and suffix the input.
    // See `check_expression_ast` for how the AST is verified.
    let (prefix, suffix) = if matches!(kind, MdxExpressionKind::AttributeExpression) {
        ("({", "})")
    } else {
        ("", "")
    };

    let (file, syntax, version) = create_config(format!("{}{}{}", prefix, value, suffix));
    let mut errors = vec![];
    let result = parse_file_as_expr(&file, syntax, version, None, &mut errors);

    match result {
        Err(error) => swc_error_to_signal(&error, value.len(), prefix.len(), "expression"),
        Ok(tree) => {
            if errors.is_empty() {
                let place = fix_swc_position(tree.span().hi.to_usize(), prefix.len());
                let result = check_expression_ast(&tree, kind);
                if matches!(result, MdxSignal::Ok) {
                    whitespace_and_comments(place, value)
                } else {
                    result
                }
            } else {
                if errors.len() > 1 {
                    unreachable!("parse_expression: todo: multiple errors? {:?}", errors);
                }
                swc_error_to_signal(&errors[0], value.len(), prefix.len(), "expression")
            }
        }
    }
}

/// Parse ESM in MDX with SWC.
/// To do: figure out how to fix positional info.
/// See `drop_span` in `swc_ecma_utils` for inspiration?
#[allow(dead_code)]
pub fn parse_expression_to_tree(
    value: &str,
    kind: &MdxExpressionKind,
) -> Result<Box<swc_ecma_ast::Expr>, String> {
    // For attribute expression, a spread is needed, for which we have to prefix
    // and suffix the input.
    // See `check_expression_ast` for how the AST is verified.
    let (prefix, suffix) = if matches!(kind, MdxExpressionKind::AttributeExpression) {
        ("({", "})")
    } else {
        ("", "")
    };

    let (file, syntax, version) = create_config(format!("{}{}{}", prefix, value, suffix));
    let mut errors = vec![];
    let result = parse_file_as_expr(&file, syntax, version, None, &mut errors);

    match result {
        Err(error) => Err(swc_error_to_string(&error)),
        Ok(expr) => {
            if errors.is_empty() {
                if matches!(kind, MdxExpressionKind::AttributeExpression) {
                    let mut obj = None;

                    if let swc_ecma_ast::Expr::Paren(d) = *expr {
                        if let swc_ecma_ast::Expr::Object(d) = *d.expr {
                            obj = Some(d)
                        }
                    };

                    if let Some(mut obj) = obj {
                        if obj.props.len() > 1 {
                            Err("Unexpected extra content in spread: only a single spread is supported".into())
                        } else if let Some(swc_ecma_ast::PropOrSpread::Spread(d)) = obj.props.pop()
                        {
                            Ok(d.expr)
                        } else {
                            Err("Unexpected prop in spread: only a spread is supported".into())
                        }
                    } else {
                        Err("Expected an object spread (`{...spread}`)".into())
                    }
                } else {
                    Ok(expr)
                }
            } else {
                if errors.len() > 1 {
                    println!(
                        "parse_expression_to_tree: todo: multiple errors? {:?}",
                        errors
                    );
                }
                Err(swc_error_to_string(&errors[0]))
            }
        }
    }
}

/// Serialize an SWC module.
/// To do: support comments.
#[allow(dead_code)]
pub fn serialize(module: &Module) -> String {
    let mut buf = vec![];
    let cm = Lrc::new(SourceMap::new(FilePathMapping::empty()));
    // let comm = &program.comments as &dyn swc_common::comments::Comments;
    {
        let mut emitter = Emitter {
            cfg: swc_ecma_codegen::Config {
                ..Default::default()
            },
            cm: cm.clone(),
            // To do: figure out how to pass them.
            comments: None,
            wr: JsWriter::new(cm, "\n", &mut buf, None),
        };

        emitter.emit_module(module).unwrap();
    }

    String::from_utf8_lossy(&buf).to_string()
}

/// Check that the resulting AST of ESM is OK.
///
/// This checks that only module declarations (import/exports) are used, not
/// statements.
fn check_esm_ast(tree: &Module) -> MdxSignal {
    let mut index = 0;
    while index < tree.body.len() {
        let node = &tree.body[index];

        if !node.is_module_decl() {
            let place = fix_swc_position(node.span().hi.to_usize(), 0);
            return MdxSignal::Error(
                "Unexpected statement in code: only import/exports are supported".to_string(),
                place,
            );
        }

        index += 1;
    }

    MdxSignal::Ok
}

/// Check that the resulting AST of an expressions is OK.
///
/// This checks that attribute expressions are the expected spread.
fn check_expression_ast(tree: &Expr, kind: &MdxExpressionKind) -> MdxSignal {
    if matches!(kind, MdxExpressionKind::AttributeExpression)
        && tree
            .unwrap_parens()
            .as_object()
            .and_then(|object| {
                if object.props.len() == 1 {
                    object.props[0].as_spread()
                } else {
                    None
                }
            })
            .is_none()
    {
        MdxSignal::Error(
            "Expected a single spread value, such as `...x`".to_string(),
            0,
        )
    } else {
        MdxSignal::Ok
    }
}

/// Turn an SWC error into an `MdxSignal`.
///
/// * If the error happens at `value_len`, yields `MdxSignal::Eof`
/// * Else, yields `MdxSignal::Error`.
fn swc_error_to_signal(
    error: &SwcError,
    value_len: usize,
    prefix_len: usize,
    name: &str,
) -> MdxSignal {
    let place = fix_swc_position(error.span().hi.to_usize(), prefix_len);
    let message = format!(
        "Could not parse {} with swc: {}",
        name,
        swc_error_to_string(error)
    );

    if place >= value_len {
        MdxSignal::Eof(message)
    } else {
        MdxSignal::Error(message, place)
    }
}

/// Turn an SWC error into a string.
fn swc_error_to_string(error: &SwcError) -> String {
    error.kind().msg().into()
}

/// Move past JavaScript whitespace (well, actually ASCII whitespace) and
/// comments.
///
/// This is needed because for expressions, we use an API that parses up to
/// a valid expression, but there may be more expressions after it, which we
/// don’t alow.
fn whitespace_and_comments(mut index: usize, value: &str) -> MdxSignal {
    let bytes = value.as_bytes();
    let len = bytes.len();
    let mut in_multiline = false;
    let mut in_line = false;

    while index < len {
        // In a multiline comment: `/* a */`.
        if in_multiline {
            if index + 1 < len && bytes[index] == b'*' && bytes[index + 1] == b'/' {
                index += 1;
                in_multiline = false;
            }
        }
        // In a line comment: `// a`.
        else if in_line {
            if index + 1 < len && bytes[index] == b'\r' && bytes[index + 1] == b'\n' {
                index += 1;
                in_line = false;
            } else if bytes[index] == b'\r' || bytes[index] == b'\n' {
                in_line = false;
            }
        }
        // Not in a comment, opening a multiline comment: `/* a */`.
        else if index + 1 < len && bytes[index] == b'/' && bytes[index + 1] == b'*' {
            index += 1;
            in_multiline = true;
        }
        // Not in a comment, opening a line comment: `// a`.
        else if index + 1 < len && bytes[index] == b'/' && bytes[index + 1] == b'/' {
            index += 1;
            in_line = true;
        }
        // Outside comment, whitespace.
        else if bytes[index].is_ascii_whitespace() {
            // Fine!
        }
        // Outside comment, not whitespace.
        else {
            return MdxSignal::Error(
                "Could not parse expression with swc: Unexpected content after expression"
                    .to_string(),
                index,
            );
        }

        index += 1;
    }

    if in_multiline {
        MdxSignal::Error(
            "Could not parse expression with swc: Unexpected unclosed multiline comment, expected closing: `*/`".to_string(),
            index,
        )
    } else if in_line {
        // EOF instead of EOL is specifically not allowed, because that would
        // mean the closing brace is on the commented-out line
        MdxSignal::Error(
            "Could not parse expression with swc: Unexpected unclosed line comment, expected line ending: `\\n`".to_string(),
            index,
        )
    } else {
        MdxSignal::Ok
    }
}

/// Create configuration for SWC, shared between ESM and expressions.
///
/// This enables modern JavaScript (ES2022) + JSX.
fn create_config(source: String) -> (SourceFile, Syntax, EsVersion) {
    (
        // File.
        SourceFile::new(
            FileName::Anon,
            false,
            FileName::Anon,
            source,
            BytePos::from_usize(1),
        ),
        // Syntax.
        Syntax::Es(EsConfig {
            jsx: true,
            ..EsConfig::default()
        }),
        // Version.
        EsVersion::Es2022,
    )
}

/// Turn an SWC byte position from a resulting AST to an offset in the original
/// input string.
fn fix_swc_position(index: usize, prefix_len: usize) -> usize {
    index - 1 - prefix_len
}