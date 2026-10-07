use std::collections::HashMap;

use crate::{Error, Result};

/// Caller-owned variables, independent of process environment or dotenv.
pub type Variables = HashMap<String, String>;

/// Expand the source template once; inserted values are never evaluated again.
pub(crate) fn expand(
    text: &str,
    variables: &Variables,
) -> Result<String> {
    expand_with(text, variables, |_, value, _| Ok(value.to_owned()))
}

#[cfg(feature = "json")]
pub(crate) fn expand_json(
    text: &str,
    variables: &Variables,
) -> Result<String> {
    expand_with(text, variables, |name, value, quoted| {
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
    mut render: impl FnMut(&str, &str, bool) -> Result<String>,
) -> Result<String> {
    let mut output = String::new();
    let mut remaining = text;
    let mut quoted = false;
    let mut escaped = false;
    while let Some(ch) = remaining.chars().next() {
        if ch != '$' {
            output.push(ch);
            remaining = &remaining[ch.len_utf8()..];
            if escaped {
                escaped = false;
            } else if quoted && ch == '\\' {
                escaped = true;
            } else if ch == '"' {
                quoted = !quoted;
            }
            continue;
        }
        if remaining.starts_with("$$") {
            output.push('$');
            remaining = &remaining[2..];
            escaped = false;
            continue;
        }
        let (name, consumed) =
            if let Some(expression) = remaining.strip_prefix("${") {
                let end = expression.find('}').ok_or(Error::InvalidVariable)?;
                let name = &expression[..end];
                if !valid_name(name) {
                    return Err(Error::InvalidVariable);
                }
                (name, end + 3)
            } else {
                let expression = &remaining[1..];
                if !expression
                    .as_bytes()
                    .first()
                    .is_some_and(|ch| ch.is_ascii_alphabetic() || *ch == b'_')
                {
                    output.push('$');
                    remaining = expression;
                    escaped = false;
                    continue;
                }
                let end = expression
                    .bytes()
                    .take_while(|ch| ch.is_ascii_alphanumeric() || *ch == b'_')
                    .count();
                (&expression[..end], end + 1)
            };
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

fn valid_name(name: &str) -> bool {
    name.as_bytes()
        .first()
        .is_some_and(|ch| ch.is_ascii_alphabetic() || *ch == b'_')
        && name.bytes().all(|ch| ch.is_ascii_alphanumeric() || ch == b'_')
}
