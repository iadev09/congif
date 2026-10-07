#![cfg(feature = "json")]

use congif::{ConfigReader, Error, Variables};
use serde::Deserialize;
use std::path::PathBuf;

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures").join(name)
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Settings {
    text: String,
    suffix: String,
    number: u64,
    enabled: bool,
    optional: Option<u64>,
    literal: String,
    escaped_quote: String,
}

#[tokio::test]
async fn json_template_preserves_string_content_and_scalar_types() {
    let text = "quote: \"; slash: \\; newline: \n; unicode: ç; $OTHER";
    let variables = Variables::from([
        ("TEXT".into(), text.into()),
        ("ROOT".into(), "/srv/example".into()),
        ("NUMBER".into(), "3".into()),
        ("ENABLED".into(), "true".into()),
        ("OPTIONAL".into(), "null".into()),
    ]);
    let reader = ConfigReader::new().with_builtin_parsers().unwrap();
    let config: Settings = reader
        .read_with_variables(fixture("template.json"), &variables)
        .await
        .unwrap();
    assert_eq!(config.text, text);
    assert_eq!(config.suffix, "/srv/example_backup");
    assert_eq!(config.number, 3);
    assert!(config.enabled);
    assert_eq!(config.optional, None);
    assert_eq!(config.literal, "$TEXT/$20/$TEXT");
    assert_eq!(config.escaped_quote, format!("quote: \"{text}"));
}

#[tokio::test]
async fn unquoted_json_variables_cannot_insert_strings_or_document_structure() {
    let reader = ConfigReader::new().with_builtin_parsers().unwrap();
    for value in
        ["example", "\"example\"", "{}", "[]", "1, \"extra\": true", "NaN"]
    {
        let result = reader
            .read_with_variables::<congif::ConfigValue>(
                fixture("template-number.json"),
                &Variables::from([("VALUE".into(), value.into())]),
            )
            .await;
        assert!(
            matches!(result, Err(Error::InvalidVariableValue(name)) if name == "VALUE")
        );
    }
    let result = reader
        .read_with_variables::<congif::ConfigValue>(
            fixture("template-number.json"),
            &Variables::new(),
        )
        .await;
    assert!(
        matches!(result, Err(Error::UndefinedVariable(name)) if name == "VALUE")
    );
}
