# arbitrary-almanac

A personal almanac. One binary, workflows in Fennel, built on
[Almanac](https://github.com/ivy/almanac).

Part of Arbitrary Definitions, my umbrella for self-hosted, single-user tools.
Almanac is ivy's library for automation that is deterministic up to the point
of inference: it receives events, gathers facts through recorded calls, applies
a pure policy, and produces a small closed set of effects. Each almanac is a
Rust binary built from workflows written against the library.

arbitrary-almanac uses the same engine in a different shape. The Rust is one
binary that ships every provider, event, and prompt, and is built once. The
behaviour is Fennel, bundled into that binary per deployment, so changing a
workflow never means touching Rust.

## What it is

- **One generic binary.** Almanac's runtime, store, outbox, and replay, with
  every provider compiled in: GitHub, the Claude harness, the Podman sandbox,
  package registries, and (planned) Quire. It is released on its own schedule,
  the way frork is.
- **Workflows in Fennel.** Each workflow is a `.fnl` file with a declaration,
  a `gather`, and a `policy`. It runs on Almanac's real runtime through the
  `Registered` trait, the seam that ivy's
  [Fennel spike](https://github.com/ivy/almanac/pull/5) used.
- **Capabilities, not names.** A workflow receives what it may use as
  arguments and can reach nothing else. `gather` gets its providers and
  agents. `policy` gets facts and effect constructors, with no clock, no
  randomness, and no I/O.
- **A build step.** `arbitrary-almanac build` checks the scripts and appends
  them to the generic binary with a trailer, as `deno compile` does. The
  checks that the spike ran at load run here instead, so a bad workflow fails
  on my machine before anything ships. Bundling compiles no Rust, so a Mac
  produces a Linux artifact without a cross toolchain.
- **Almanac's safety model, kept.** Hostile text is never a Lua string: it
  lives in an opaque userdata with a closed `taint` table. Policy runs in a
  fresh, pure VM. Iteration order and printed addresses are patched for
  determinism, so a replay repeats a decision without calling a model.

## A workflow

The intended shape. The syntax is not final.

```fennel
(local declare
  {:name :build-failures
   :on :github.check_suite.completed
   :intent "Repair a Renovate update whose build fails"
   :may [:attest :ask :push]
   :runs-per-hour 30
   :agents {:fixer {:prompt :fixer :vars [:repo]}}})

(fn gather [{: github : ages : fixer} event]
  ...)

(fn policy [facts {: attest : ask : push}]
  (case facts
    {:kind :nothing} []
    {:kind :repaired : patch} [(push patch)]
    ...))

{: declare : gather : policy}
```

The declaration is a literal, so it is read without running anything. The
manifest is what the declaration and the destructured arguments say. A run
refuses any provider, agent, or effect the manifest does not list.

## Using it

```
arbitrary-almanac build workflows/ -o almanac   # check, then bundle
./almanac check                                 # providers and secrets
./almanac synth                                 # print the manifests
./almanac run                                   # a dry run by default: records, applies nothing
ALMANAC_MODE=live ./almanac run                 # live is deliberate, set in the environment
```

Everything after `build` is Almanac's own CLI, unchanged. `build` is a verb on
the same binary.

## Design principles

arbitrary-almanac keeps Almanac's principles except the second, which it
restates:

- **Behaviour is code, bundled into the binary.** Almanac says workflows are
  compiled in. Here they are bundled in, and checked at build. Runtime
  configuration still only sets limits and addresses, and can only lower what
  the bundle declares.
- **The host is paid for once.** Rust changes when the catalog of events,
  providers, or prompts changes, not when a workflow does.
- **Authority comes from arguments.** If a workflow was not handed something,
  it cannot name it. This replaces most of the spike's bans on macros,
  `require`, and computed targets.
- **Data where possible.** Fennel is at its best when the file is mostly one
  table. Fact kinds are declared as data, so `{:kind ...}` and policy clauses
  are checked for exhaustiveness at build.
- **One household.** Built for me, run on my machines. No multi-tenancy, no
  SaaS.

## Layout

```
Cargo.toml          depends on Almanac's crates by git
src/
  lib.rs            the library root
  catalog.rs        every event, provider, and prompt the binary ships
  host/             the Fennel host, promoted from the spike
  bundle.rs         the trailer: write at build, read at startup
  bin/arbitrary-almanac/
    main.rs         almanac::main_with, plus the build verb
prompts/            agent prompts, built into the binary
workflows/          .fnl workflows, bundled per deployment
```

## Roadmap

1. **Run the spike's workflow from here.** Depend on Almanac by git, promote
   the spike's host, and run `build_failures.fnl` against GitHub in a dry run.
   No changes to Almanac. Fix `read-logs` on the way, since a bare `case`
   pattern binds `false` from a passing job.
2. **Capabilities and the bundle.** Hand providers and effects in as
   arguments, retire the tree-walking analyzer, and add `build`.
3. **Quire as a source.** Quire already posts a `PushEvent` to `quire-ci`. A
   broker `Source` for Quire needs Quire to sign it and add the broker as a
   target. CI results would need a new event from Quire.
4. **SQLite store.** Almanac's `Store` is a trait, but `app.rs` connects to
   Postgres directly, as do the broker and `deliveries`. This needs a change
   upstream.
5. **Effects without pull requests.** Almanac's `Push` and `Ask` assume a
   pull request, and Quire has none. Until a workflow needs this,
   pull-request work stays on the GitHub mirror.

A `Tracker` for Linear or Ranger fits anywhere: the trait is two methods.

## Status

Planning. Nothing here runs yet. The case for this shape is in my
[review of the Fennel spike](https://github.com/ivy/almanac/pull/5).