# Congif

## Ownership

- Providers own config structs, Serde rules, custom deserializers, validation
  and any domain-specific compilation. Their config code belongs in their own
  modules. Congif must not learn host, cache, database or language semantics.
- `ConfigReader` owns parser registration and async source reading. `new()` is
  empty; `with_builtin_parsers()` adds all enabled built-in parsers together.
  `ConfigParser` receives the original path and bytes, then returns an owned,
  format-neutral Serde value. The reader deserializes it into the provider's
  requested type. Custom adapters may carry application execution context.
- `ConfigStore<T>` owns one active immutable snapshot. An application can
  aggregate all provider config types inside `T` and publish them in one swap.
  It does not allocate a separate store for each field or provider.
- Continuo and applications own lifecycle ordering and runtime preparation.
  Congif's optional watcher reports source changes to an owner-supplied handler;
  it does not read, validate, publish or reload on that owner's behalf. There
  is no periodic work or implicit timeout.

`ConfigReader::bind_environment::<Owner>` stores the owner's env binding;
providers declare config sources and may override their binding through
`Configurable::environment()`. Watchers use `sources_for` to include the
bound environment ancestry. Each preparation batch pins these bindings.
Environment preparation distinguishes startup, source invalidation and explicit
reload. Startup uses Dotenw's registered snapshots. Source invalidation carries
changed paths to `prepare_changed_sources`: config-only changes reuse the current
environment catalog, while registered env paths refresh their dependent branches.
An env-only notification whose effective owner snapshot is unchanged skips typed
configuration preparation entirely. Native duplicates still require source
verification; no timer, quiet-period assumption or dropped notification is used.
Explicit `prepare_changed_owners` refreshes the selected owners' environment
chains before parsing. Config equality retains existing typed snapshots; the
application/runtime decides whether to execute a reload regardless of equality.

## Read and publication

1. Configurable components declare one `ConfigFile` with an explicit `Format`.
   The reader resolves that registered parser before opening the source. Direct
   `read(path)` calls still select by extension. Paths have no implicit base directory.
2. The reader expands `$NAME` and `${NAME}` in UTF-8 source text using the supplied
   variables or environment snapshot, without reading process environment.
   Expansion is single-pass and parser-owned: JSON escapes quoted insertions
   and restricts unquoted insertions to numbers, booleans and null; other
   adapters default to literal substitution. The parser then reads syntax or
   executes its own adapter behavior. Provider Serde rules decode the resulting
   value into the candidate config.
3. The application assembles the complete candidate and prepares any required
   provider-owned derived data. The store's `replace` accepts a caller-supplied
   validation function; it validates before changing the active snapshot.
4. One atomic swap publishes the complete value. Existing `Snapshot<T>`
   handles keep the previous generation alive.

Read, parse, schema and validation errors do not publish a partial config.
Preparation batches collect affected owners' declared sources before their
`configure` calls. File bytes are pinned once. Format-neutral syntax trees are
shared within that batch by source path, parser identity and expanded content; each owner still
deserializes its own schema and validates its own domain. Bound environments
allow syntax preparation before configure. Direct reader calls can supply
explicit variables; declared component sources use their registered environment.
Different expanded content never shares a syntax tree. The cache ends with the
batch; it is not another active configuration store. Unchanged environment-only
notifications still skip unaffected owners before any source preparation.
The initial store value must already be prepared by its owner. Concurrent
successful replacements publish in swap order; callers that need reload
request ordering coordinate it in their lifecycle owner.

Pin one snapshot at the beginning of an operation. Loading a new snapshot for
each field can mix generations even though every individual load is atomic.
Interior-mutability fields are allowed by Rust but defeat immutable-snapshot
semantics if mutated after publication; providers must preserve that boundary.

An atomic config swap is not an atomic runtime-resource reload. Ports, pools,
tasks and other live resources remain provider-owned. Their lifecycle must
prepare and coordinate application of a candidate separately.

## Extensibility

Serde `derive` and `rc` are enabled. YAML, TOML, JSON, INI and XML adapter types are
optional features with no formats enabled by default. Enabled built-in adapters
are registered together with `with_builtin_parsers()`; individual adapters can also
be registered by parser value.
Parsers declare their own supported extensions.
Duplicate normalized extensions are rejected, not overwritten, and a failed
registration publishes none of the new parser's mappings.
Each built-in adapter lives in its own `src/parsers/` module. The always-present
`parsers::ConfigParser` contract and `ConfigSource` live in `parsers/custom.rs`;
custom adapters do not require any built-in format feature.
The core has no PHP or Laravel dependency. Claviron can implement
`ConfigParser` and supply its PHP execution and Laravel bootstrap context.

Tests exercise parsing, schema rejection, registration and snapshot behavior.
They do not boot providers, servers or applications, and use no sleeps.

## Typed configuration lifecycle

`Configurable` declares its associated Serde `Config` type and one `source()`.
The reader reads, deserializes and validates before returning a candidate.
Preparation never invokes lifecycle callbacks. After the entire batch is prepared,
`configure(&Self::Config)` applies startup resources. `configure_registered` pins
and acknowledges the candidate only on success. Providers do not implement a
separate owner trait: `ConfigRegistration` is an erased lifecycle adapter, not
another storage owner. Config snapshots belong to Congif.

`ConfigObserver` is the optional typed application hook. Its object-safe owner
adapter exposes observation without a second registry: `Some` installs source
watching, `None` does not. The runtime prepares the candidate batch before
calling `config_changed(&Self::Config)`. Congif pins that exact candidate and
acknowledges application only after the observer succeeds. This callback is not
executed by parsing, validation or the native notification thread. Manual reload
remains a separate lifecycle operation.

## Optional native watching

The `watch` feature exposes `watch::ConfigWatcher`. One watcher serves all
registrations. `register::<Config>(paths, handler)` addresses a concrete config
owner through `ConfigChanged<Config>`; multiple owners of the same type are
allowed without a duplicate service registry or implicit provider resolution.
The registration retains only paths and the handler, not a provider instance.
The caller can capture a weak provider reference or enqueue a Continuo signal.

Watcher support is restricted to Linux (inotify), macOS (FSEvents) and FreeBSD
(kqueue). Both the module and optional notify dependency are target-gated.
Enabling `watch` on another OS produces an explicit compile error; disabling
it imposes no watcher-specific OS restriction on core. No polling fallback or
debounce duration is introduced. Parent
directories are watched non-recursively so atomic file replacements remain
visible. File symlinks are explicitly unsupported. Replacing a watched parent
directory requires recreating the watcher. Parent paths are canonicalized at
registration, while the source file may not yet exist.

Callbacks execute outside registration locks on the notification thread and
should only submit work. Access events are ignored; a rescan indication notifies
all source owners. Backend errors go to the supplied error handler. Events can
repeat and do not guarantee a writer has finished; the owner controls admission,
validation and reload ordering. Dropping the watcher releases all registrations.
Tests exercise event dispatch directly, without native watcher boot or sleeps.
