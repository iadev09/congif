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
2. The reader expands `${NAME}` in UTF-8 source text using the supplied
   variables or environment snapshot, without reading process environment.
   Expansion is single-pass and parser-owned: JSON escapes quoted insertions
   and restricts unquoted insertions to numbers, booleans and null. Only JSON
   also accepts bare `$NAME` for complete unquoted value tokens, avoiding
   `${NAME}`'s structural-brace ambiguity in JSON tooling. Bare dollar patterns
   inside strings and in every other adapter remain literal. The parser then
   reads syntax or executes its own adapter behavior. Provider Serde rules
   decode the resulting value into the candidate config.
3. The application assembles the complete candidate and prepares any required
   provider-owned derived data. The store's `replace` accepts a caller-supplied
   validation function; it validates before changing the active snapshot.
4. One atomic swap publishes the complete value. Existing `Snapshot<T>`
   handles keep the previous generation alive.

Read, parse, schema and validation errors do not publish a partial config.
`Error::Validation` is a provider's verdict on a candidate (`Error::validation`
wraps an error value, `Error::invalid` a sentence); `Error::Message` is every
other refusal — an apply step, a poisoned lock, an owner never prepared — and
is never dressed as validation.
Preparation batches collect affected owners' declared sources before their
`configure` calls. File bytes are pinned once. Format-neutral syntax trees are
shared within that batch by source path, parser identity, expanded content and
the pinned variable values (including absence); each owner still
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
Registration accepts a shared reader reference, so hosts can add adapters during
composition after placing the reader in application state. A short write lock
protects catalog updates; reads retain an Arc to the adapter and never hold that
lock across execution. Each preparation batch pins its complete parser catalog.
Each built-in adapter lives in its own `src/parsers/` module. The always-present
`parsers::ConfigParser` contract and `ConfigSource` live in `parsers/custom.rs`;
custom adapters do not require any built-in format feature.
The core has no PHP or Laravel dependency. Claviron can implement
`ConfigParser` and supply its PHP execution and Laravel bootstrap context.
`parse_with_variables` delivers the reader's pinned environment separately from
the source. Its default delegates to `parse`, preserving existing adapters.
An executable adapter can keep its text intact in `expand_variables` and expose
variables through language APIs instead of injecting values into executable code.
Two identical source texts with different variable values never share that
adapter's result within a batch; equal variable maps do share it.

Tests exercise parsing, schema rejection, registration and snapshot behavior.
They do not boot providers, servers or applications, and use no sleeps.

## Typed configuration lifecycle

`Configurable` declares its associated Serde `Config` type and one `source()`.
The reader reads, deserializes and validates before returning a candidate.
A source that does not exist is a read error unless the owner answers
`absent_source()` with the configuration that stands in its place; that value is
validated like a read one, and a batch skips pinning the missing file so the
owner, not the batch, decides. `congif::export` turns any serializable config
into the exported value tree for owners that expose it whole.
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

`PartialConfigObserver` is a separate opt-in extension of `ConfigObserver`.
Only owners exposing it through `as_partial_config_observer` register partial
sources with `partial_sources(&ConfigReader)`. On a partial-file notification,
the reader passes the changed registered paths to `read_partial_config`, not
to the ordinary full-document reader. The owner retains unchanged prepared
parts and returns a complete candidate for the existing validation/equality
pipeline. Congif owns no domain-specific merge or per-file cache.
Changed paths retain the spelling registered by the owner. The reader normalizes
them only for matching absolute watcher notifications, not for looking up the
owner's prepared source parts. Relative declarations therefore remain relative
when delivered to the partial reader and application hook.

After preparation, `partial_config_changed` receives the exact candidate and
its changed paths; this hook does not parse or reopen sources. A failed apply
remains pending, including its partial paths, so a subsequent candidate cannot
lose earlier unapplied changes. Root changes, effective environment changes,
startup and explicit reload use full preparation and the normal application
hook. Owners without this capability retain their existing API and behavior.

## Optional native watching

### Program environments

An owner may run programs — a PHP application or script, a task's process —
each started with an environment of its own, declared in the owner's
configuration as `env_file` entries and inline values. Such an owner
implements `Environmental`: `program_environments()` lists the declarations
from the applied configuration, and `environment_changed(program, snapshot)`
is told when one of a program's files was re-read. The runtime asks after
`configure` and after a reload and hands the list to
`ConfigReader::prepare_programs(owner, programs)`, which keeps one dotenw node
per `(owner, program)`: registered once, `replace`d when the declaration
changed, never a node per run. Each node is detached — only what its files and
values declare is in the snapshot, while `${NAME}` expands from the owner's
bound configuration environment — and layered like the reader's own `.env`:
`<file>` then `<file>.<environment>`, both registered whether or not they
exist yet, so a file that appears later is read. `program_snapshot(owner,
program)` is what a program is started with; `program_sources(owner)` are the
paths the owner's watch registration adds beside its configuration sources.
A notification for one of them goes through the same `prepare_changed_sources`
as the root document: the store refreshes every node reading that file once,
and `take_program_changes()` hands the runtime the `(owner, program,
snapshot)` triples to deliver. `refresh_programs(owner)` re-reads a reload.

### Include dependencies

The YAML and TOML adapters reserve `includes` (`include` is a compatibility
alias) throughout their value trees. A declaration deserializes as one `PathBuf`
or a list of them; malformed values and empty paths fail before schema decoding.
The parser exposes this metadata through `ConfigParser::includes`. Other
adapters opt in by implementing that hook. Built-in adapters consume the reserved
keys before schema decoding, so even strict schemas need no metadata fields.
The batch syntax cache pins both the cleaned value and its dependency metadata.
Domain-specific merging is not implemented by Congif. An owner whose document
is completed by its includes merges them in `Configurable::read_config`, the
hook `prepare` reads through: it takes `includes_for` its source and reads each
file with `ConfigReader::read_include`, which parses under the owner's
environment and records that file's own includes. The merged value is the
owner's `Config`, so equality — and therefore "changed" — covers the includes.

`ConfigReader` owns `HashMap<PathBuf, Vec<PathBuf>>` for these dependencies.
Relative entries are resolved beside their declaring source. Successful file
syntax reads replace their list, including removal of old declarations, before
schema decoding. Watch routing is not active runtime configuration: rejected
schema/validation retains all typed snapshots but follows newly discovered paths
so their correction triggers another attempt. Unreadable syntax retains the last
known list; the root source remains watched. Submitted `parse` calls do not
modify live dependencies. Included documents are
not opened merely to discover dependencies, so dependencies declared inside
them are the responsibility of their own parser/loader.

`sources_for` combines this list with source and environment dependencies.
Opt-in partial owners may add recursively discovered dependencies through
`partial_sources`; registration itself opens no included documents.
Include notifications participate in the same source-invalidation check as the
root document. `ConfigWatcher::update_owner_paths` replaces dependency routing
without adding another callback or watcher. Continuo refreshes routing after
startup preparation and watched attempts, including rejected candidates; removed dependencies stop
notifying their old owner. Glob patterns resolve to existing concrete files on
each source read; they are never passed to the watcher as selectors. A file
created later is discovered by rereading the declaring source, not by watching
its directory as an application dependency. Directory matches are excluded.
Congif does not expose directory subscriptions. Native parent-directory watches
remain an implementation detail to observe atomic replacement of named files.

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
