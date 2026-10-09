#![cfg(feature = "json")]

use congif::{ConfigParser, ConfigReader, ConfigSource, Error, Variables};
use serde::Deserialize;
use std::path::{Path, PathBuf};

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

#[tokio::test]
async fn bare_json_variables_preserve_scalar_types_and_quoted_dollar_patterns()
{
    #[derive(Debug, Deserialize)]
    struct Scalars {
        count: u64,
        enabled: bool,
        optional: Option<u64>,
        values: Vec<f64>,
        literal: String,
        expanded: String,
    }
    let variables = Variables::from([
        ("COUNT".into(), "4".into()),
        ("ENABLED".into(), "true".into()),
        ("OPTIONAL".into(), "null".into()),
        ("DECIMAL".into(), "-1.25".into()),
        ("EXPONENT".into(), "2e3".into()),
        ("TEXT".into(), "quote: \"; ${COUNT}; $COUNT".into()),
    ]);
    let text = r#"{
        "count":$COUNT,
        "enabled": $ENABLED,
        "optional": $OPTIONAL,
        "values": [$DECIMAL, ${COUNT}, $EXPONENT],
        "literal": "$COUNT/$uri/$20",
        "expanded": "${TEXT}"
    }"#;
    let parsed: Scalars = ConfigReader::new()
        .with_builtin_parsers()
        .unwrap()
        .parse_with_variables(
            ConfigSource {
                path: Path::new("example.json"),
                bytes: text.as_bytes(),
            },
            &variables,
        )
        .await
        .unwrap();
    assert_eq!(parsed.count, 4);
    assert!(parsed.enabled);
    assert_eq!(parsed.optional, None);
    assert_eq!(parsed.values, [-1.25, 4.0, 2000.0]);
    assert_eq!(parsed.literal, "$COUNT/$uri/$20");
    assert_eq!(parsed.expanded, variables["TEXT"]);
    assert_eq!(
        congif::parsers::JsonParser
            .expand_variables("$COUNT", &variables)
            .unwrap(),
        "4"
    );
}

#[test]
fn bare_json_variables_reject_undefined_values_and_non_scalar_injection() {
    let parser = congif::parsers::JsonParser;
    for value in
        ["example", "\"example\"", "{}", "[]", "1, \"extra\": true", "NaN", ""]
    {
        let result = parser.expand_variables(
            r#"{"value": $VALUE}"#,
            &Variables::from([("VALUE".into(), value.into())]),
        );
        assert!(
            matches!(result, Err(Error::InvalidVariableValue(name)) if name == "VALUE")
        );
    }
    let result =
        parser.expand_variables(r#"{"value": $VALUE}"#, &Variables::new());
    assert!(
        matches!(result, Err(Error::UndefinedVariable(name)) if name == "VALUE")
    );
}

#[tokio::test]
async fn bare_json_variables_must_be_complete_value_tokens() {
    let variables = Variables::from([("VALUE".into(), "2".into())]);
    let reader = ConfigReader::new().with_builtin_parsers().unwrap();
    for text in [
        r#"{"value": 1$VALUE}"#,
        r#"{"value": $VALUE.5}"#,
        r#"{"value": $VALUE-1}"#,
        r#"{"value": $1VALUE}"#,
    ] {
        let result = reader
            .parse_with_variables::<congif::ConfigValue>(
                ConfigSource {
                    path: Path::new("example.json"),
                    bytes: text.as_bytes(),
                },
                &variables,
            )
            .await;
        assert!(
            matches!(result, Err(Error::Parse { .. })),
            "{text}: {result:?}"
        );
    }
}

#[test]
fn non_json_adapters_keep_bare_script_and_redirect_patterns_literal() {
    let parsers: &[&dyn ConfigParser] = &[
        #[cfg(feature = "yaml")]
        &congif::parsers::YamlParser,
        #[cfg(feature = "toml")]
        &congif::parsers::TomlParser,
        #[cfg(feature = "ini")]
        &congif::parsers::IniParser,
        #[cfg(feature = "xml")]
        &congif::parsers::XmlParser,
    ];
    let text = "entry_script: $SCRIPT\nredirect: https://example.test$request_uri\ncount: $COUNT";
    let variables = Variables::from([
        ("SCRIPT".into(), "must-not-expand".into()),
        ("request_uri".into(), "must-not-expand".into()),
        ("COUNT".into(), "2".into()),
    ]);
    for parser in parsers {
        assert_eq!(parser.expand_variables(text, &variables).unwrap(), text);
        assert_eq!(
            parser.expand_variables(text, &Variables::new()).unwrap(),
            text
        );
    }
}
