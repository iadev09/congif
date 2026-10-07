# Congif

Make your application and its services configurable.

Give each service a typed configuration, its sources and validation rules.
Congif reads and prepares those configurations, keeps their current snapshots
and can notify your runtime when a source changes. With runtime integration,
those notifications can reload the affected services without restarting the
application.

Invalid configuration keeps the previous snapshots intact. Your services own
how accepted configuration changes are applied to their running resources.

Use it for a single application's settings or a shared configuration layer
across many services. There is no required framework or service registry;
[Continuo](https://github.com/iadev09/continuo) supplies a runtime integration.

## Read a configuration

~~~toml
[dependencies]
congif = { version = "0.1", features = ["json"] }
serde = { version = "1", features = ["derive"] }
~~~

Enable the formats you use, add their built-in parsers together, then read into
your own type:

~~~rust
use congif::ConfigReader;
use serde::Deserialize;
use std::net::SocketAddr;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct HttpConfig {
    listen: SocketAddr,
}

async fn load() -> congif::Result<HttpConfig> {
    let reader = ConfigReader::new().with_builtin_parsers()?;
    reader.read("config/http.json").await
}
~~~

~~~json
{
  "listen": "127.0.0.1:3000"
}
~~~

`ConfigReader::new()` creates an empty reader. `with_builtin_parsers()` registers all
built-in parsers whose features are enabled. The source file's extension selects the parser, so one
reader can read JSON, YAML and TOML files together. Paths are used as supplied,
without an implicit base directory. Unsupported extensions are rejected before
the source is opened; duplicate custom parser registrations are errors.

Each parser declares the extensions it handles; callers do not supply a
separate format name. `YamlParser`, for example, handles both `.yaml` and `.yml`.

For already-read content, `parse` accepts a `ConfigSource` containing the source
path and bytes, without opening a file.

## Supply variables explicitly

~~~rust
let variables = congif::Variables::from([
    ("HTTP_LISTEN".into(), "127.0.0.1:3000".into()),
]);

let config: HttpConfig = reader
    .read_with_variables("config/http.json", &variables)
    .await?;
~~~

~~~json
{
  "listen": "${HTTP_LISTEN}"
}
~~~

Congif expands `$NAME` and `${NAME}` in source text first, parses the resulting document,
then deserializes into your schema. It does not read process environment or
dotenv files itself.

Expansion is single-pass: inserted values are never recursively expanded.
Names use ASCII letters, digits and underscores, starting with a letter or
underscore. `$$` inserts a literal dollar; `$20` remains literal text.
Undefined variables and malformed expressions are errors.

The JSON adapter tracks quoted strings and escapes inserted string content.
Unquoted placeholders must contain a JSON number, boolean or `null`; they cannot
insert strings, objects, arrays or additional fields. The caller's schema still
validates the final type and range. Other adapters use literal substitution;
supply values appropriate to their format, or override `expand_variables` in a
custom parser. Quote strings and leave numeric or boolean placeholders unquoted:

~~~json
{"listen": "$HTTP_LISTEN", "max_connections": $HTTP_MAX_CONNECTIONS}
~~~

## Configure an application component

`Configurable` connects reading to an owner without requiring a registry or
a running service:

~~~rust
use congif::{ConfigFile, Configurable, Error, Format, Result, async_trait};
use serde::Deserialize;
use std::path::PathBuf;

#[derive(Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
struct CounterConfig {
    increment: u64,
}

struct CounterSettings {
    path: PathBuf,
}

#[async_trait]
impl Configurable for CounterSettings {
    type Config = CounterConfig;

    fn source(&self) -> ConfigFile {
        ConfigFile { path: self.path.clone(), format: Format::Json }
    }

    fn namespace(&self) -> String {
        "counter".into()
    }

    async fn configure(&self, config: &Self::Config) -> Result<()> {
        // Apply the already-validated configuration to component resources.
        println!("counter increment: {}", config.increment);
        Ok(())
    }

    fn validate_config(&self, config: &Self::Config) -> Result<()> {
        if config.increment == 0 {
            return Err(Error::invalid("increment must be greater than zero"));
        }
        Ok(())
    }
}
~~~

`reader.prepare(&owner).await` reads and validates without publishing or calling
`configure`. Each component declares one source and its explicit format, independent
of its filename extension. After batch preparation, `configure_registered(&owner)`
applies the typed candidate and acknowledges it only on success. Continuo drives
these steps before running services. Validation must not mutate live resources.

`namespace` defaults to the component type name in snake_case. Typed snapshots
are addressed by component type; dot-path lookup uses explicitly exported fields.

Environment bindings may be supplied by application composition:

~~~rust
reader.bind_environment::<CounterSettings>(environment_handle)?;
~~~

Preparation uses that environment when expanding the declared source.
`reader.sources_for(&owner)` includes both config files and environment ancestry
for watchers. Alternatively, a provider may return its registered handle from
`Configurable::environment()`. This takes precedence over the application binding
and is pinned for the whole preparation batch; it is not a child process's env.
`reader.parse_for::<CounterSettings>(source).await?` parses submitted source with
the same binding without publishing it; the owner then validates it before saving.

## Publish a batch together

~~~rust
reader.prepare_owners(&[&counter_settings, &http_settings]).await?;
let counter = reader.snapshot::<CounterSettings>()?;
~~~

Each component retains its concrete schema. An erased registration adapter lets
runtimes orchestrate heterogeneous components; Congif owns the snapshots. The
catalog is indexed by component type, not by configuration type.

`reader.snapshot::<Owner>()` returns the current typed `Arc` directly: no file
read, parsing or serialization. Keep it for the duration of an operation;
reload publishes a new generation without changing the one already pinned.
The explicit reader keeps separate applications' configuration stores isolated.

### Observe prepared configuration changes

Implement `ConfigObserver` and return `Some(self)` from
`Configurable::as_config_observer()` to opt into automatic source notifications.
The default is `None`: declaring or reading a source does not request a watch.
The runtime delivers the validated `&Self::Config` to `config_changed`, outside
the native watcher thread. Preparation alone never calls this handler.

`reader.notify_changed(&owner).await?` pins the prepared generation, invokes
the handler and acknowledges it only on success. A failure keeps the candidate
unapplied for retry. This is separate from a manual runtime reload request.

### Dot-path access for hosted applications

Rust integrations can expose configuration to PHP, Lua, JavaScript or other
hosted applications through `Configurable::export_config`. The owner returns
an optional `ConfigValue` tree of the fields intended for those consumers.
That tree is prepared and published with the typed snapshot, not reconstructed
on every lookup. Owners which do not export remain typed-only.

~~~rust
let listen = reader.config::<std::net::SocketAddr>("http.listen")?;
// Result<Option<SocketAddr>>: missing key is None; invalid conversion is Err.
let raw = reader.value("http.listen")?; // For a language bridge's native values.
~~~

Dot paths traverse string-keyed maps and numeric sequence indices. No default
is supplied. Namespaces must be unique; an export or namespace error rejects
the entire candidate batch. Prefer `reader.snapshot::<Owner>()` in Rust
when the owner is known: it shares the typed `Arc` without value conversion.

Implement `HasConfigReader` on an application's state to use the same lookup
directly, without passing its reader as a parameter:

~~~rust
use congif::HasConfigReader;

let listen = state.get_config::<std::net::SocketAddr>("http.listen")?;
~~~

The trait delegates to the application's own reader; no global store, runtime
provider or lifecycle dependency is introduced.

For a small application-facing API, wrap `get_config` in an inherent state
method. Only the state module needs the trait import; handlers do not:

~~~rust
// state.rs
use congif::{ConfigReader, HasConfigReader};
use serde::de::DeserializeOwned;

struct AppState {
    reader: ConfigReader,
}

impl HasConfigReader for AppState {
    fn config_reader(&self) -> &ConfigReader {
        &self.reader
    }
}

impl AppState {
    pub fn config<T: DeserializeOwned>(&self, key: &str) -> congif::Result<Option<T>> {
        self.get_config(key)
    }
}

// A handler: no HasConfigReader import required.
let listen = state.config::<std::net::SocketAddr>("http.listen")?;
~~~

This is demonstrated live by Continuo's `examples/playground`: its `/config`
endpoint returns `http.listen`, `counter.increment` and a missing key (`null`).
Change a registered config or environment file, then request `/config` again
to see the newly prepared snapshot. These values describe prepared configuration,
not necessarily the live resources of a service which rejected its reload.

During one preparation batch:

- File bytes are shared by canonical path: the same source is read once.
- Owners run in the order supplied by the caller.
- Later owners can resolve configurations already prepared in that batch.
- All successful candidates are published in one catalog swap.
- A read, parse or validation failure leaves the previous catalog unchanged.

The caller supplies dependency order. Congif does not construct a service graph
or invoke lifecycle hooks. Multiple instances of the same owner type share one
catalog slot; this is not an instance-keyed configuration registry.

### Prepared versus applied

Applications can pin `reader.prepared_config(owner)` before applying resources
and call `reader.confirm_applied(token).await` only after success. The applied
and prepared records share the same `Arc` when they agree; there is no second
copy of the config. Confirming an older token never acknowledges a newer
candidate, and a token from another reader is rejected.

Once applied tracking is established, change preparation compares candidates
with that applied generation. A rejected resource update remains pending even
when a later notification produces the same candidate. Reverting to the applied
settings updates the prepared catalog without requesting a resource reload.
Congif compares configurations through their `PartialEq` implementation; owners
do not implement a separate comparison hook. Derive equality for ordinary
schemas, or implement it on the config type when domain-specific equality is
needed. Consumers which do not confirm application retain the original
prepared-to-prepared comparison behavior.

Continuo's config-watch integration confirms the boot configuration when its
control loop starts and acknowledges successful reloads. Providers keep their
runtime resources, not an extra whole config just for comparison. Manual reload
requests remain explicit operations even if configuration is unchanged.

A batch pins each file when first read. It is not an atomic filesystem snapshot
across multiple files, nor a transaction across sockets, pools or other runtime
resources.

## Keep a standalone snapshot

For a single configuration or an application-owned aggregate, use `ConfigStore<T>`:

~~~rust
let store = congif::ConfigStore::new(initial_config);
let pinned = store.snapshot();

store.replace(candidate, |config| validate(config))?;

// The pinned snapshot still refers to the previous configuration.
~~~

The initial value must already be prepared. `replace` validates before swapping
and returns the previous snapshot. Existing readers keep their generation alive.

Pin one snapshot for an operation rather than loading each field separately.
Treat published values as immutable: interior mutation defeats snapshot
isolation even though Rust permits it.

## Watch sources, without owning reload

The optional `watch` feature provides one native watcher for registered sources:

~~~rust
use congif::watch::ConfigWatcher;
use std::path::PathBuf;

let mut watcher = ConfigWatcher::new(|error| {
    // Forward the backend error to your application's diagnostics.
    report(error);
})?;

watcher.register::<CounterSettings>(
    [PathBuf::from("config/counter.json")],
    |change| {
        // Forward invalidation to your application's lifecycle owner.
        notify_counter(change.paths);
    },
)?;
~~~

`report` and `notify_counter` above represent application-owned handlers.
Keep the watcher alive for as long as notifications are needed.

Multiple owners can watch the same file. Parent-directory watches are shared
within a `ConfigWatcher`, and matching registrations receive source changes.
`owners_for_paths` returns the affected owner types.

Watching the parent observes file creation, removal and editor atomic
replacement. File symlinks are rejected. Replacing a watched parent directory
requires recreating the watcher.

Callbacks run on the native notification thread: forward the change rather than
blocking that thread. Watching does not automatically read, publish or reload.
There is no debounce timer, polling fallback or implicit retry cadence.

Native watching supports Linux, macOS and FreeBSD. Enabling `watch` on another
target produces a compile error; the reader itself does not require watching.

## Add a parser

Implement `ConfigParser` in your own crate and declare its supported file
extensions through `extensions`. Register the parser itself, not a string tag.
The parser receives the original path and bytes and returns a format-neutral
`ConfigValue`. Congif supplies already-expanded source text and then applies
the caller's Serde schema.

Custom parsers do not require a built-in format feature. They can carry their
own execution context: a PHP adapter, for example, belongs in the application
that supplies its PHP runtime, not in Congif.

## Features

No parsers or watchers are enabled by default.

| Feature | Registered parser type or capability |
| --- | --- |
| `json` | `parsers::JsonParser` |
| `yaml` | `parsers::YamlParser` |
| `toml` | `parsers::TomlParser` |
| `ini` | `parsers::IniParser` |
| `xml` | `parsers::XmlParser` |
| `watch` | `watch::ConfigWatcher` |

Enabling a format makes its parser available. `with_builtin_parsers()` registers all
enabled built-ins together; `register()` adds an individual built-in or custom
parser. Neither operation silently replaces an existing mapping.
Serde's `derive` and `rc` features are enabled by Congif.

## Working alongside other crates

[Continuo](https://github.com/iadev09/continuo) uses this configuration contract
to coordinate preparation and service reload through its runtime.
[Claviron](https://github.com/iadev09/claviron) is the application-server project
these independent crates are being developed alongside.

Congif depends on neither project. Configuration ownership and runtime lifecycle
are separate responsibilities.

## Why the name?

A misspelling habit.

## License

Licensed under either of:

- MIT license ([LICENSE-MIT](LICENSE-MIT))
- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE))
