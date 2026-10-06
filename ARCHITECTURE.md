# Congif

## Ownership

- Providers own config structs, Serde rules, custom deserializers, validation
  and any domain-specific compilation. Their config code belongs in their own
  modules. Congif must not learn host, cache, database or language semantics.
- `ConfigReader` owns explicit parser registration and async source reading.
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

## Read and publication

1. The reader resolves the explicitly registered extension before opening the
   source. Paths are used as supplied, without an implicit base directory.
2. The parser reads syntax or executes its own adapter behavior. Provider
   Serde rules decode the resulting value into the candidate config.
3. The application assembles the complete candidate and prepares any required
   provider-owned derived data. The store's `replace` accepts a caller-supplied
   validation function; it validates before changing the active snapshot.
4. One atomic swap publishes the complete value. Existing `Snapshot<T>`
   handles keep the previous generation alive.

Read, parse, schema and validation errors do not publish a partial config.
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
optional features with no default parser selection. Adapters are registered
explicitly; duplicate normalized extensions are rejected, not overwritten.
Each built-in adapter lives in its own `src/parsers/` module. The always-present
`parsers::ConfigParser` contract and `ConfigSource` live in `parsers/custom.rs`;
custom adapters do not require any built-in format feature.
The core has no PHP or Laravel dependency. Claviron can implement
`ConfigParser` and supply its PHP execution and Laravel bootstrap context.

Tests exercise parsing, schema rejection, registration and snapshot behavior.
They do not boot providers, servers or applications, and use no sleeps.

## Configuration owner prototype

`Configurable` declares its associated `Config` type and prepares it through
`configure(&ConfigReader)`. `ConfigReader::prepare` also runs the owner's
`validate_config` before returning that typed candidate. It never publishes
the candidate or starts runtime resources. There is no registry dependency,
and the associated type need not implement Serde: the owner can construct it
from one or more separately deserialized inputs. Multi-stage lifecycle design
is not fixed by this prototype.

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
