use std::collections::HashMap;

use crate::{Error, Result};

/// Caller-owned variables, independent of process environment or dotenv.
pub type Variables = HashMap<String, String>;

/// Expand the source template once; inserted values are never evaluated again.
/// Only `${NAME}` is a variable: a bare `$name` is text, as it is in a
/// redirect target or a shell-looking path, and `$$` writes a literal `$`
/// where `${` would otherwise start an expression.
pub(crate) fn expand(
    text: &str,
    variables: &Variables,
) -> Result<String> {
    expand_with(text, variables, Comments::None, |_, value, _| {
        Ok(value.to_owned())
    })
}

/// A format's comment syntax. Text inside a comment is copied as it is: a
/// `${NAME}` written there is documentation, not a variable.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[allow(dead_code)]
pub(crate) enum Comments {
    /// The format has no comments (JSON).
    None,
    /// `#` to end of line, at line start or after whitespace (YAML, TOML).
    Hash,
    /// `;` or `#` to end of line (INI).
    HashOrSemicolon,
    /// `<!-- ... -->` (XML).
    Xml,
}

/// Expand everything outside the format's comments.
///
/// Which adapters exist is a feature choice, so with few formats enabled
/// parts of this stay unused.
#[allow(dead_code)]
pub(crate) fn expand_outside_comments(
    text: &str,
    variables: &Variables,
    comments: Comments,
) -> Result<String> {
    expand_with(text, variables, comments, |_, value, _| Ok(value.to_owned()))
}

#[cfg(feature = "json")]
pub(crate) fn expand_json(
    text: &str,
    variables: &Variables,
) -> Result<String> {
    expand_with(text, variables, Comments::None, |name, value, quoted| {
        if quoted {
            let encoded = serde_json::to_string(value)
                .expect("string serialization is infallible");
            return Ok(encoded[1..encoded.len() - 1].to_owned());
        }
        match serde_json::from_str::<serde_json::Value>(value) {
            Ok(
                serde_json::Value::Number(_)
                | serde_json::Value::Bool(_)
                | serde_json::Value::Null,
            ) => Ok(value.to_owned()),
            _ => Err(Error::InvalidVariableValue(name.to_owned())),
        }
    })
}

fn expand_with(
    text: &str,
    variables: &Variables,
    comments: Comments,
    mut render: impl FnMut(&str, &str, bool) -> Result<String>,
) -> Result<String> {
    let mut output = String::new();
    let mut remaining = text;
    let mut quoted = false;
    let mut single_quoted = false;
    let mut escaped = false;
    // Whether a line comment may start here: at the text's start, a line's
    // start, or after whitespace.
    let mut at_boundary = true;
    while let Some(ch) = remaining.chars().next() {
        if !quoted
            && !single_quoted
            && let Some(rest) = comment_span(remaining, comments, at_boundary)
        {
            let span = &remaining[..remaining.len() - rest.len()];
            output.push_str(span);
            remaining = rest;
            at_boundary = true;
            continue;
        }
        if ch != '$' {
            output.push(ch);
            remaining = &remaining[ch.len_utf8()..];
            if escaped {
                escaped = false;
            } else if quoted && ch == '\\' {
                escaped = true;
            } else if ch == '"' && !single_quoted {
                quoted = !quoted;
            } else if ch == '\'' && !quoted && comments != Comments::None {
                single_quoted = !single_quoted;
            }
            at_boundary = ch.is_whitespace();
            continue;
        }
        at_boundary = false;
        if remaining.starts_with("$$") {
            output.push('$');
            remaining = &remaining[2..];
            escaped = false;
            continue;
        }
        let Some(expression) = remaining.strip_prefix("${") else {
            // A bare `$` is text: `$uri` in a redirect, `$20` in a price.
            output.push('$');
            remaining = &remaining[1..];
            escaped = false;
            continue;
        };
        let end = expression.find('}').ok_or(Error::InvalidVariable)?;
        let name = &expression[..end];
        if !valid_name(name) {
            return Err(Error::InvalidVariable);
        }
        let consumed = end + 3;
        if escaped {
            return Err(Error::InvalidVariable);
        }
        let value = variables
            .get(name)
            .ok_or_else(|| Error::UndefinedVariable(name.to_owned()))?;
        output.push_str(&render(name, value, quoted)?);
        remaining = &remaining[consumed..];
    }
    Ok(output)
}

/// The comment that starts at `text`, returned as the text after it; `None`
/// when no comment starts here.
fn comment_span(
    text: &str,
    comments: Comments,
    at_boundary: bool,
) -> Option<&str> {
    let line_comment = |markers: &[char]| {
        let ch = text.chars().next()?;
        (at_boundary && markers.contains(&ch))
            .then(|| text.find('\n').map_or("", |end| &text[end..]))
    };
    match comments {
        Comments::None => None,
        Comments::Hash => line_comment(&['#']),
        Comments::HashOrSemicolon => line_comment(&['#', ';']),
        Comments::Xml => {
            let rest = text.strip_prefix("<!--")?;
            Some(rest.find("-->").map_or("", |end| &rest[end + 3..]))
        }
    }
}

fn valid_name(name: &str) -> bool {
    name.as_bytes()
        .first()
        .is_some_and(|ch| ch.is_ascii_alphabetic() || *ch == b'_')
        && name.bytes().all(|ch| ch.is_ascii_alphanumeric() || ch == b'_')
}
