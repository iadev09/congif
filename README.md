# Congif

Typed configuration reading, validated preparation and atomic snapshots for Rust.

Your application owns the schema and its rules. Congif reads sources through
registered parsers, deserializes them with Serde and stores prepared
configuration without taking ownership of services or runtime resources.

## Read a configuration

~~~toml
[dependencies]
congif = { version = "0.1", features = ["json"] }
serde = { version = "1", features = ["derive"] }
~~~

Register the format you use, then read into your own type:

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
    let mut reader = ConfigReader::new();
    reader.register("json", congif::parsers::JsonParser)?;
    reader.read("config/http.json").await
}
~~~

~~~json
{
  "listen": "127.0.0.1:3000"
}
~~~

There is no implicit parser selection or base directory. Paths are used as
supplied. Unsupported extensions are rejected before the source is opened;
duplicate parser registrations are errors.

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

Congif parses the document first, expands `${NAME}` inside string values,
then deserializes into your schema. It does not read process environment or
dotenv files itself.

Expansion does not change mapping keys, recursively expand inserted values or
coerce strings into numbers. Undefined variables and malformed expressions
are errors. JSON variables therefore belong inside quoted strings.

## Let an owner prepare its configuration

`Configurable` connects reading to an owner without requiring a registry or
a running service:

~~~rust
use congif::{ConfigReader, Configurable, Error, Result, async_trait};
use serde::Deserialize;
use std::path::PathBuf;

#[derive(Deserialize)]
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

    fn sources(&self) -> Vec<PathBuf> {
        vec![self.path.clone()]
    }

    fn namespace(&self) -> String {
        "counter".into()
    }

    async fn configure(&self, reader: &ConfigReader) -> Result<Self::Config> {
        reader.read(&self.path).await
    }

    fn validate_config(&self, config: &Self::Config) -> Result<()> {
        if config.increment == 0 {
            return Err(Error::validation(std::io::Error::other(
                "increment must be greater than zero",
            )));
        }
        Ok(())
    }
}
~~~

`reader.prepare(&owner).await` reads and validates a candidate without publishing
it. The owner can select sources, parsers and variables, and prepare its own
derived configuration. It must not mutate live runtime resources while doing so.

`sources` declares files for optional watching. `namespace` defaults to the
unqualified owner type name in snake_case and can be overridden. It does not
implement a string-key lookup API; snapshots are addressed by owner type.

## Publish a batch together

~~~rust
reader.prepare_owners(&[&counter_settings, &http_settings]).await?;
let counter = reader.snapshot::<CounterSettings>()?;
~~~

Each owner retains its concrete schema through the object-safe `ConfigOwner`
adapter. The catalog is indexed by owner type, not by configuration type.

During one preparation batch:

- File bytes are shared by canonical path: the same source is read once.
- Owners run in the order supplied by the caller.
- Later owners can resolve configurations already prepared in that batch.
- All successful candidates are published in one catalog swap.
- A read, parse or validation failure leaves the previous catalog unchanged.

The caller supplies dependency order. Congif does not construct a service graph
or invoke lifecycle hooks. Multiple instances of the same owner type share one
catalog slot; this is not an instance-keyed configuration registry.

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

Implement `ConfigParser` in your own crate and register it under an extension.
The parser receives the original path and bytes and returns a format-neutral
`ConfigValue`. Congif then applies variables and the caller's Serde schema.

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

Enabling a format makes its parser available; registration is still explicit.
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
