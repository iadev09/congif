use std::path::Path;
use std::sync::Arc;

#[cfg(any(
    feature = "json",
    feature = "yaml",
    feature = "toml",
    feature = "ini",
    feature = "xml"
))]
fn fixture(name: &str) -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures").join(name)
}

use serde::{Deserialize, Serialize};

use congif::{
    BoxError, ConfigParser, ConfigReader, ConfigSource, ConfigStore,
    ConfigValue, Error, Variables, async_trait,
};

#[derive(Debug, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
struct Settings {
    cache: u32,
    http: u32,
}

struct FixtureParser;

#[async_trait]
impl ConfigParser for FixtureParser {
    async fn parse(
        &self,
        _: ConfigSource<'_>,
    ) -> Result<ConfigValue, BoxError> {
        Ok(serde_value::to_value(Settings { cache: 2, http: 2 })?)
    }
}

#[tokio::test]
async fn external_parser_produces_owned_typed_config() {
    let mut reader = ConfigReader::new();
    reader.register("fixture", FixtureParser).unwrap();
    let settings: Settings = reader
        .parse(ConfigSource { path: Path::new("settings.FIXTURE"), bytes: b"" })
        .await
        .unwrap();
    assert_eq!(settings, Settings { cache: 2, http: 2 });
}

#[test]
fn duplicate_parser_does_not_replace_the_registered_adapter() {
    let mut reader = ConfigReader::new();
    reader.register("fixture", FixtureParser).unwrap();
    assert!(matches!(
        reader.register("FIXTURE", FixtureParser),
        Err(Error::DuplicateParser(_))
    ));
    assert!(matches!(
        reader.register(".fixture", FixtureParser),
        Err(Error::InvalidExtension(_))
    ));
}

#[tokio::test]
async fn unknown_format_is_rejected_before_io() {
    let reader = ConfigReader::new();
    assert!(matches!(
        reader.read::<Settings>("missing.unregistered").await,
        Err(Error::UnsupportedFormat { .. })
    ));
}

#[test]
fn one_swap_publishes_the_whole_config_and_retains_pinned_readers() {
    let store = ConfigStore::new(Settings { cache: 1, http: 1 });
    let pinned = store.snapshot();
    let previous =
        store.replace(Settings { cache: 2, http: 2 }, |_| Ok(())).unwrap();
    assert_eq!(*pinned, Settings { cache: 1, http: 1 });
    assert_eq!(*previous, *pinned);
    assert_eq!(*store.snapshot(), Settings { cache: 2, http: 2 });
}

#[test]
fn provider_validation_failure_keeps_the_active_snapshot() {
    let store = ConfigStore::new(Settings { cache: 1, http: 1 });
    let rejected = store.replace(Settings { cache: 0, http: 2 }, |candidate| {
        if candidate.cache == 0 {
            Err(Error::validation(std::io::Error::other(
                "cache must be positive",
            )))
        } else {
            Ok(())
        }
    });
    assert!(matches!(rejected, Err(Error::Validation(_))));
    assert_eq!(*store.snapshot(), Settings { cache: 1, http: 1 });
}

#[test]
fn snapshot_clone_does_not_require_config_clone() {
    let store = Arc::new(ConfigStore::new(Settings { cache: 1, http: 1 }));
    let pinned = store.snapshot();
    let cloned = pinned.clone();
    assert!(std::ptr::eq(pinned.as_ref(), cloned.as_ref()));
    fn shared<T: Send + Sync>(_: &T) {}
    shared(&store);
}

#[cfg(feature = "json")]
#[tokio::test]
async fn provider_serde_rules_reject_unknown_fields() {
    let mut reader = ConfigReader::new();
    reader.register("json", congif::parsers::JsonParser).unwrap();
    let result = reader.read::<Settings>(fixture("unknown-field.json")).await;
    assert!(matches!(result, Err(Error::Deserialize { .. })));
}

#[cfg(all(feature = "yaml", feature = "toml", feature = "json"))]
#[tokio::test]
async fn formats_share_the_same_provider_schema() {
    let mut reader = ConfigReader::new();
    reader.register("yaml", congif::parsers::YamlParser).unwrap();
    reader.register("toml", congif::parsers::TomlParser).unwrap();
    reader.register("json", congif::parsers::JsonParser).unwrap();
    for path in ["settings.yaml", "settings.toml", "settings.json"] {
        let settings: Settings = reader.read(fixture(path)).await.unwrap();
        assert_eq!(settings, Settings { cache: 2, http: 2 });
    }
}

#[cfg(all(feature = "yaml", feature = "toml", feature = "json"))]
#[tokio::test]
async fn variables_are_resolved_after_parsing_in_all_formats() {
    #[derive(Debug, Deserialize, PartialEq)]
    struct Paths {
        directory: String,
        port: String,
        literal: String,
    }

    let mut reader = ConfigReader::new();
    reader.register("json", congif::parsers::JsonParser).unwrap();
    reader.register("yaml", congif::parsers::YamlParser).unwrap();
    reader.register("toml", congif::parsers::TomlParser).unwrap();

    let variables = Variables::from([
        ("APP_ROOT".into(), "/srv/example".into()),
        ("PORT".into(), "8080".into()),
        ("TEXT".into(), "quote: \"; slash: \\; newline: \n; ${OTHER}".into()),
    ]);
    // Inserted values are not reparsed or recursively expanded.
    for path in ["paths.json", "paths.yaml", "paths.toml"] {
        let parsed: Paths = reader
            .read_with_variables(fixture(path), &variables)
            .await
            .unwrap();
        assert_eq!(parsed.directory, "/srv/example/storage", "{path}");
        assert_eq!(parsed.port, "8080", "{path}");
        assert_eq!(parsed.literal, variables["TEXT"], "{path}");
    }
}

#[cfg(feature = "json")]
#[tokio::test]
async fn json_rejects_unquoted_variable_placeholders() {
    let mut reader = ConfigReader::new();
    reader.register("json", congif::parsers::JsonParser).unwrap();
    let result =
        reader.read::<ConfigValue>(fixture("unquoted-variable.json")).await;
    assert!(matches!(result, Err(Error::Parse { .. })));
}

#[cfg(feature = "json")]
#[tokio::test]
async fn quoted_json_variable_is_not_implicitly_a_number() {
    #[derive(Debug, Deserialize)]
    struct Port {
        #[serde(rename = "port")]
        _port: u16,
    }

    let mut reader = ConfigReader::new();
    reader.register("json", congif::parsers::JsonParser).unwrap();
    let result = reader
        .read_with_variables::<Port>(
            fixture("port.json"),
            &Variables::from([("PORT".into(), "8080".into())]),
        )
        .await;
    assert!(matches!(result, Err(Error::Deserialize { .. })));
}

struct ValueParser(ConfigValue);

#[async_trait]
impl ConfigParser for ValueParser {
    async fn parse(
        &self,
        _: ConfigSource<'_>,
    ) -> Result<ConfigValue, BoxError> {
        Ok(self.0.clone())
    }
}

async fn resolve_variables(
    value: ConfigValue,
    variables: &Variables,
) -> congif::Result<ConfigValue> {
    let mut reader = ConfigReader::new();
    reader.register("fixture", ValueParser(value))?;
    reader
        .parse_with_variables(
            ConfigSource { path: Path::new("value.fixture"), bytes: b"" },
            variables,
        )
        .await
}

#[tokio::test]
async fn variables_reject_missing_and_malformed_expressions() {
    let variables = Variables::new();
    let missing = ConfigValue::String("${MISSING}".into());
    assert!(
        matches!(resolve_variables(missing, &variables).await, Err(Error::UndefinedVariable(name)) if name == "MISSING")
    );
    for text in ["${}", "${NAME", "${{NAME}"] {
        let value = ConfigValue::String(text.into());
        assert!(matches!(
            resolve_variables(value, &variables).await,
            Err(Error::InvalidVariable)
        ));
    }
}

#[tokio::test]
async fn variables_visit_nested_values_but_preserve_keys_and_bare_dollars() {
    let variables = Variables::from([("NAME".into(), "example".into())]);
    let value = ConfigValue::Map(std::collections::BTreeMap::from([(
        ConfigValue::String("${NAME}".into()),
        ConfigValue::Seq(vec![ConfigValue::String(
            "${NAME}/${NAME}/$NAME".into(),
        )]),
    )]));
    let value = resolve_variables(value, &variables).await.unwrap();
    let ConfigValue::Map(values) = value else { panic!("expected map") };
    assert_eq!(
        values[&ConfigValue::String("${NAME}".into())],
        ConfigValue::Seq(vec![ConfigValue::String(
            "example/example/$NAME".into()
        )])
    );
}

#[cfg(feature = "ini")]
#[tokio::test]
async fn ini_sections_and_variables_follow_the_provider_schema() {
    #[derive(Debug, Deserialize, PartialEq)]
    struct Config {
        title: String,
        paths: Paths,
    }
    #[derive(Debug, Deserialize, PartialEq)]
    struct Paths {
        directory: String,
        port: String,
    }
    let mut reader = ConfigReader::new();
    reader.register("ini", congif::parsers::IniParser).unwrap();
    let variables = Variables::from([("ROOT".into(), "/srv/example".into())]);
    let parsed: Config = reader
        .read_with_variables(fixture("settings.ini"), &variables)
        .await
        .unwrap();
    assert_eq!(
        parsed,
        Config {
            title: "Example".into(),
            paths: Paths {
                directory: "/srv/example/storage".into(),
                port: "8080".into()
            }
        }
    );
}

#[cfg(feature = "ini")]
#[tokio::test]
async fn ini_rejects_duplicate_properties_and_malformed_sections() {
    let mut reader = ConfigReader::new();
    reader.register("ini", congif::parsers::IniParser).unwrap();
    for path in ["duplicate.ini", "malformed.ini"] {
        assert!(matches!(
            reader.read::<ConfigValue>(fixture(path)).await,
            Err(Error::Parse { .. })
        ));
    }
}

#[cfg(feature = "xml")]
#[tokio::test]
async fn xml_elements_attributes_sequences_and_variables_are_typed() {
    #[derive(Debug, Deserialize)]
    struct Config {
        directory: String,
        endpoint: Vec<Endpoint>,
    }
    #[derive(Debug, Deserialize)]
    struct Endpoint {
        #[serde(rename = "@name")]
        name: String,
        #[serde(rename = "$text")]
        address: String,
    }
    let mut reader = ConfigReader::new();
    reader.register("xml", congif::parsers::XmlParser).unwrap();
    let variables =
        Variables::from([("ROOT".into(), "quote: \" & slash: \\".into())]);
    let parsed: Config = reader
        .read_with_variables(fixture("settings.xml"), &variables)
        .await
        .unwrap();
    assert_eq!(parsed.directory, format!("{}/storage", variables["ROOT"]));
    assert_eq!(parsed.endpoint.len(), 2);
    assert_eq!(parsed.endpoint[0].name, "first");
    assert_eq!(parsed.endpoint[0].address, "https://example.com/?a=1&b=2");
}

#[cfg(feature = "xml")]
#[tokio::test]
async fn xml_rejects_malformed_documents_dtd_namespaces_and_mixed_content() {
    let mut reader = ConfigReader::new();
    reader.register("xml", congif::parsers::XmlParser).unwrap();
    for path in ["malformed.xml", "dtd.xml", "namespace.xml", "mixed.xml"] {
        assert!(matches!(
            reader.read::<ConfigValue>(fixture(path)).await,
            Err(Error::Parse { .. })
        ));
    }
}
