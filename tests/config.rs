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
    fn extensions(&self) -> &[&str] {
        &["fixture"]
    }

    async fn parse(
        &self,
        _: ConfigSource<'_>,
    ) -> Result<ConfigValue, BoxError> {
        Ok(serde_value::to_value(Settings { cache: 2, http: 2 })?)
    }
}

#[tokio::test]
async fn external_parser_produces_owned_typed_config() {
    let mut reader = ConfigReader::new().with_builtin_parsers().unwrap();
    reader.register(FixtureParser).unwrap();
    let settings: Settings = reader
        .parse(ConfigSource { path: Path::new("settings.FIXTURE"), bytes: b"" })
        .await
        .unwrap();
    assert_eq!(settings, Settings { cache: 2, http: 2 });
}

#[test]
fn duplicate_parser_does_not_replace_the_registered_adapter() {
    let mut reader = ConfigReader::new().with_builtin_parsers().unwrap();
    reader.register(FixtureParser).unwrap();
    assert!(matches!(
        reader.register(AliasParser(&["FIXTURE"])),
        Err(Error::DuplicateParser(_))
    ));
    assert!(matches!(
        reader.register(AliasParser(&[".fixture"])),
        Err(Error::InvalidExtension(_))
    ));
}

struct AliasParser(&'static [&'static str]);

#[async_trait]
impl ConfigParser for AliasParser {
    fn extensions(&self) -> &[&str] {
        self.0
    }

    async fn parse(
        &self,
        source: ConfigSource<'_>,
    ) -> Result<ConfigValue, BoxError> {
        FixtureParser.parse(source).await
    }
}

#[tokio::test]
async fn parser_declared_aliases_select_the_same_adapter() {
    let mut reader = ConfigReader::new().with_builtin_parsers().unwrap();
    reader.register(AliasParser(&["fixture", "custom", "CUSTOM"])).unwrap();
    for path in ["settings.fixture", "settings.CUSTOM"] {
        let settings: Settings = reader
            .parse(ConfigSource { path: Path::new(path), bytes: b"" })
            .await
            .unwrap();
        assert_eq!(settings, Settings { cache: 2, http: 2 });
    }
}

#[tokio::test]
async fn rejected_alias_registration_installs_no_partial_mapping() {
    let mut reader = ConfigReader::new().with_builtin_parsers().unwrap();
    reader.register(FixtureParser).unwrap();
    assert!(matches!(
        reader.register(AliasParser(&["custom", "FIXTURE"])),
        Err(Error::DuplicateParser(_))
    ));
    assert!(matches!(
        reader.register(AliasParser(&["custom", ".invalid"])),
        Err(Error::InvalidExtension(_))
    ));
    assert!(matches!(
        reader.register(AliasParser(&[])),
        Err(Error::MissingExtensions)
    ));
    assert!(matches!(
        reader
            .parse::<Settings>(ConfigSource {
                path: Path::new("settings.custom"),
                bytes: b"",
            })
            .await,
        Err(Error::UnsupportedFormat { .. })
    ));
}

#[tokio::test]
async fn unknown_format_is_rejected_before_io() {
    let reader = ConfigReader::new().with_builtin_parsers().unwrap();
    assert!(matches!(
        reader.read::<Settings>("missing.unregistered").await,
        Err(Error::UnsupportedFormat { .. })
    ));
}

#[cfg(feature = "yaml")]
#[tokio::test]
async fn builtins_select_both_yaml_extensions_without_individual_registration()
{
    let reader = ConfigReader::new().with_builtin_parsers().unwrap();
    let bytes = std::fs::read(fixture("settings.yaml")).unwrap();
    for path in ["settings.yaml", "settings.YML"] {
        let settings: Settings = reader
            .parse(ConfigSource { path: Path::new(path), bytes: &bytes })
            .await
            .unwrap();
        assert_eq!(settings, Settings { cache: 2, http: 2 });
    }
}

#[cfg(feature = "json")]
#[test]
fn enabled_builtin_parser_cannot_be_replaced_by_custom_registration() {
    let mut reader = ConfigReader::new().with_builtin_parsers().unwrap();
    assert!(matches!(
        reader.register(AliasParser(&["JSON"])),
        Err(Error::DuplicateParser(_))
    ));
}

#[cfg(feature = "json")]
#[tokio::test]
async fn new_reader_remains_empty_even_when_format_features_are_enabled() {
    for reader in [ConfigReader::new(), ConfigReader::default()] {
        assert!(matches!(
            reader.read::<Settings>("missing.json").await,
            Err(Error::UnsupportedFormat { .. })
        ));
    }
}

#[cfg(feature = "json")]
#[test]
fn adding_builtins_rejects_conflicting_custom_parser_mappings() {
    let mut reader = ConfigReader::new();
    reader.register(AliasParser(&["json"])).unwrap();
    assert!(matches!(
        reader.with_builtin_parsers(),
        Err(Error::DuplicateParser(_))
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
            Err(Error::invalid("cache must be positive"))
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
    let reader = ConfigReader::new().with_builtin_parsers().unwrap();
    let result = reader.read::<Settings>(fixture("unknown-field.json")).await;
    assert!(matches!(result, Err(Error::Deserialize { .. })));
}

#[cfg(all(feature = "yaml", feature = "toml", feature = "json"))]
#[tokio::test]
async fn formats_share_the_same_provider_schema() {
    let reader = ConfigReader::new().with_builtin_parsers().unwrap();
    for path in ["settings.yaml", "settings.toml", "settings.json"] {
        let settings: Settings = reader.read(fixture(path)).await.unwrap();
        assert_eq!(settings, Settings { cache: 2, http: 2 });
    }
}

#[cfg(all(feature = "yaml", feature = "toml", feature = "json"))]
#[tokio::test]
async fn source_variables_are_resolved_before_parsing_in_all_formats() {
    #[derive(Debug, Deserialize, PartialEq)]
    struct Paths {
        directory: String,
        port: String,
        literal: String,
    }

    let reader = ConfigReader::new().with_builtin_parsers().unwrap();

    let variables = Variables::from([
        ("APP_ROOT".into(), "/srv/example".into()),
        ("PORT".into(), "8080".into()),
        ("TEXT".into(), "literal ${OTHER}".into()),
    ]);
    // Inserted values are parsed but never recursively expanded.
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
    let reader = ConfigReader::new().with_builtin_parsers().unwrap();
    let result =
        reader.read::<ConfigValue>(fixture("unquoted-variable.json")).await;
    assert!(matches!(result, Err(Error::Parse { .. })));
}

#[cfg(feature = "json")]
#[tokio::test]
async fn json_accepts_numeric_source_variables_before_parsing() {
    #[derive(Deserialize)]
    struct Port {
        port: u16,
    }
    let reader = ConfigReader::new().with_builtin_parsers().unwrap();
    let parsed: Port = reader
        .read_with_variables(
            fixture("unquoted-variable.json"),
            &Variables::from([("PORT".into(), "8080".into())]),
        )
        .await
        .unwrap();
    assert_eq!(parsed.port, 8080);
}

#[cfg(feature = "json")]
#[tokio::test]
async fn quoted_json_variable_is_not_implicitly_a_number() {
    #[derive(Debug, Deserialize)]
    struct Port {
        #[serde(rename = "port")]
        _port: u16,
    }

    let reader = ConfigReader::new().with_builtin_parsers().unwrap();
    let result = reader
        .read_with_variables::<Port>(
            fixture("port.json"),
            &Variables::from([("PORT".into(), "8080".into())]),
        )
        .await;
    assert!(matches!(result, Err(Error::Deserialize { .. })));
}

struct SourceParser;

#[async_trait]
impl ConfigParser for SourceParser {
    fn extensions(&self) -> &[&str] {
        &["fixture"]
    }

    async fn parse(
        &self,
        source: ConfigSource<'_>,
    ) -> Result<ConfigValue, BoxError> {
        Ok(ConfigValue::String(std::str::from_utf8(source.bytes)?.to_owned()))
    }
}

async fn resolve_variables(
    text: &str,
    variables: &Variables,
) -> congif::Result<ConfigValue> {
    let mut reader = ConfigReader::new().with_builtin_parsers().unwrap();
    reader.register(SourceParser)?;
    reader
        .parse_with_variables(
            ConfigSource {
                path: Path::new("value.fixture"),
                bytes: text.as_bytes(),
            },
            variables,
        )
        .await
}

#[tokio::test]
async fn variables_reject_missing_and_malformed_expressions() {
    let variables = Variables::new();
    let missing = "${MISSING}";
    assert!(
        matches!(resolve_variables(missing, &variables).await, Err(Error::UndefinedVariable(name)) if name == "MISSING")
    );
    for text in ["${}", "${NAME", "${{NAME}"] {
        assert!(matches!(
            resolve_variables(text, &variables).await,
            Err(Error::InvalidVariable)
        ));
    }
}

#[tokio::test]
async fn only_braced_names_are_variables_and_bare_dollars_stay_text() {
    let variables = Variables::from([("NAME".into(), "example".into())]);
    let value =
        resolve_variables("${NAME}/$NAME/$$NAME/$20/$${NAME}", &variables)
            .await
            .unwrap();
    assert_eq!(
        value,
        ConfigValue::String("example/$NAME/$NAME/$20/${NAME}".into())
    );
    // A redirect target keeps its placeholders; nothing is defined for them.
    let value =
        resolve_variables("https://example.test$uri", &Variables::new())
            .await
            .unwrap();
    assert_eq!(value, ConfigValue::String("https://example.test$uri".into()));
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
    let reader = ConfigReader::new().with_builtin_parsers().unwrap();
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
    let reader = ConfigReader::new().with_builtin_parsers().unwrap();
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
    let reader = ConfigReader::new().with_builtin_parsers().unwrap();
    let variables = Variables::from([("ROOT".into(), "/srv/example".into())]);
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
    let reader = ConfigReader::new().with_builtin_parsers().unwrap();
    for path in ["malformed.xml", "dtd.xml", "namespace.xml", "mixed.xml"] {
        assert!(matches!(
            reader.read::<ConfigValue>(fixture(path)).await,
            Err(Error::Parse { .. })
        ));
    }
}
