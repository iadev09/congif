use std::collections::HashMap;

use crate::{ConfigValue, Error, Result};

/// Caller-owned variables, independent of process environment or dotenv.
pub type Variables = HashMap<String, String>;

/// Expand string values only, never mapping keys or inserted variable values.
pub(crate) fn resolve(
    value: &mut ConfigValue,
    variables: &Variables,
) -> Result<()> {
    match value {
        ConfigValue::String(text) => *text = expand(text, variables)?,
        ConfigValue::Seq(values) => {
            for value in values {
                resolve(value, variables)?;
            }
        }
        ConfigValue::Map(values) => {
            for value in values.values_mut() {
                resolve(value, variables)?;
            }
        }
        ConfigValue::Option(Some(value)) | ConfigValue::Newtype(value) => {
            resolve(value, variables)?;
        }
        _ => {}
    }
    Ok(())
}

fn expand(
    text: &str,
    variables: &Variables,
) -> Result<String> {
    let mut output = String::new();
    let mut remaining = text;
    while let Some(start) = remaining.find("${") {
        output.push_str(&remaining[..start]);
        let expression = &remaining[start + 2..];
        let end = expression.find('}').ok_or(Error::InvalidVariable)?;
        let name = &expression[..end];
        if name.is_empty() || name.contains('{') {
            return Err(Error::InvalidVariable);
        }
        let value = variables
            .get(name)
            .ok_or_else(|| Error::UndefinedVariable(name.to_owned()))?;
        output.push_str(value);
        remaining = &expression[end + 1..];
    }
    output.push_str(remaining);
    Ok(output)
}
