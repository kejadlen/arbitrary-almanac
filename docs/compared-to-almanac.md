# How arbitrary-almanac differs from Almanac

Almanac is ivy's library for deterministic automation. This repository
builds on it and changes its shape: one generic binary runs workflows
written in Fennel, instead of a Rust binary per almanac. This page
explains what that change costs and buys, measured against two things:
Almanac's Rust workflows on `main`, and ivy's Fennel spike, which tried a
scripting layer inside Almanac and recommended dropping it.

This page describes design, not status. The README's roadmap says what is
built. Where a row below depends on unbuilt code, it says so, and the row
changes when the code lands. Update this page in the same commit as a
change to `src/host/` or `src/catalog.rs` that moves a row.

[spike]: https://github.com/ivy/almanac/pull/5
[principles]: https://github.com/ivy/almanac/blob/main/docs/core-principles.md

The spike is [ivy/almanac#5][spike], and its `FINDINGS.md` is the record
this page argues with. Almanac's [core principles][principles] are the
rules both approaches are measured against.

## Three approaches

Almanac, the spike, and this repository agree on the runtime, the store,
the outbox, replay, and the closed set of effects. They differ in where a
workflow lives and who enforces its rules:

| Concern | Almanac | Spike | arbitrary-almanac |
|---|---|---|---|
| Deployment unit | A binary per almanac, workflows compiled in | The same binary, scripts loaded at start | One generic binary, `.fnl` files bundled per deployment |
| Who writes Rust | Every almanac author | Every almanac author, for events and reads | This repository, once |
| Manifest | `#[workflow]` attributes plus a visitor over one function body | An analyzer walking the tree through helpers, needing literal targets | A literal `declare` plus what `gather` and `policy` destructure |
| Enforcement | The compiler, with 58 cases in `crates/fails` | 22 rules at load, 24 in a run, 2 lost | Names and shape at load, everything else in a run |
| Hostile text | `Untrusted<T>`, narrowed to a closed set | A userdata and a closed `taint` table | A userdata with no methods at all |
| Reads | `cx.observe` by convention; a raw client call compiles | A read outside `observe` is refused | The script holds no client; every call is a catalog read inside `observe` |
| Policy | A plain function, pure by signature | A fresh pure VM per call | A fresh pure VM per run, handed a constructor for each effect it destructured |
| Events | Generated types, projected by hand per almanac | 117 lines of `github.rs` per almanac | Almanac's generated types, serialized as they are |

The bundling step is planned and not built. Until it exists, the
deployment row describes the intent, and the binary loads a directory of
`.fnl` files at startup, which is the spike's shape.

## Where this repository holds more tightly than Almanac

Authority is the difference. A Rust workflow can call a client outside
`cx.observe`, or read the clock in policy, and only convention stops it.
Here `gather` receives a table of opaque handles, and each method on a
handle yields the call back to Rust, which performs it through `Cx`. The
script has no client, no clock, no `require`, and 11 globals. A read that
is not recorded cannot happen, so a replay always has what it needs.

The spike found the same thing and called it stronger than Rust. This
repository keeps it.

## Where this repository is simpler than the spike

The bans shrank. The spike inferred the manifest by walking the tree, so
it needed literal targets for every `cx.get` and `cx.observe`, banned
`require` and `lua`, and followed calls through helpers. Here the manifest
comes from what `gather` and `policy` destructure, so the walk is gone and
a workflow can pass a capability to a helper freely. A run hands each
function exactly what it destructured, so the manifest can never
understate a workflow.

What remains is a ban on nine compile-time forms (`macro`, `eval-compiler`,
`include`, `lua`, and their kin) and a top level limited to `local` and
`fn`. Both exist so the Fennel compiler runs no user code. The compiler is
Fennel 1.6.1, vendored at 7,061 lines, and it runs in the build step in
both script approaches and nowhere in Almanac.

The spike's three taint holes have no surface here. Reshaping text to turn
fixed narrowers into a reader, relabeling content with the run log's `$`
keys, and a pattern match that runs for minutes all needed methods on the
taint handle. Here the handle has none. Whether that holds depends on how
narrowing comes back, which the next section covers.

## Where Almanac holds more tightly than this repository

Feedback timing is the honest loss. Almanac fails at `cargo build`. Here a
misspelt `:kind`, a wrong fact field, or a bad argument fails only when
that branch of a run executes, and a branch runs only when its event
arrives. The spike called this the real cost and the review of it agreed.

The planned answer is the build step, which runs every load-time check
before anything ships, plus fact kinds declared as data so that `{:kind
...}` and policy clauses can be checked for exhaustiveness. Neither exists
yet, so today this repository has the spike's feedback profile.

Lost outright, with no plan to recover them:

- `Title` as a distinct type. Any clean string makes a title.
- Typed reads. A catalog read returns JSON, so a field is a lookup.
- Refactoring with a compiler behind it. Renaming a fact is a search.

## What the code has not decided yet

Taint is opaque with no way out. A workflow can carry hostile text into
its facts and back to Rust, but it cannot decide anything on it. Almanac
and the spike both have `narrow`, a trusted function that reads the raw
text and returns a value from a closed set. The spike found that every
such function is a possible way out, and every hole it found was in one.
Reintroducing `narrow` here is where that cost returns.

The catalog's reads come from Almanac's renovate almanac, which this
binary depends on as a library rather than copying. Its providers set up
GitHub, the registries, and the Claude harness from the environment, and
its own functions decide what a check suite or a pull request body means:
`github.pull` reads a Renovate pull request as it is now, `github.logs`
the tail of each failed job's log, and `ages.cooled` whether the update a
pull request's text names has cooled. The fixer runs the renovate
almanac's prompt. So this host owns only the fetching and the handing to a
script, and a judgement fixed upstream is fixed here. Agents and `push` are built, and they follow Almanac's design
rather than restating it. An agent runs a prompt the binary ships, and its
declared `:vars` must be that prompt's variables, or the workflow does not
load. A quarantined reader's answer reaches the script as an opaque handle.
A fixer reads an update only after a cooldown read in the same run has
passed: the cooldown's proof is Almanac's `Cooled`, held in Rust beside the
run and never handed to the script, so a script can neither forge nor keep
it. The fixer's patch is also a handle, and `(push ...)` checks it against
Almanac's default patch policy where it is made, so a refused patch ends
the run with its reason. A model that can veto but never approve has nothing to
compare against here yet, because no workflow asks a classifier.

## Trusted code

Every line on the security boundary is one a reviewer must trust. The
counts below exclude comments and blank lines:

| Approach | On the boundary |
|---|---|
| Almanac | 749 lines, `taint.rs` and `crates/macros` |
| Spike | 1,931 lines of Rust and 365 of Fennel and Lua |
| arbitrary-almanac | 2,020 lines of Rust, beside the renovate almanac it reads through |

This repository's tests are 1,889 lines beside that. The comparison
favours Almanac, which puts the boundary in the type system, and it is
the price of letting the operator never touch Rust.

## Why the shape changed

The spike asked whether a Rust library should gain a second language and
answered no. The review of it, which this repository follows, asked a
different question: whether Almanac is a library people write Rust
almanacs against, or one binary that operators configure with scripts. In
the second shape, two of the spike's three reasons to drop Fennel go away,
because the host is paid for once and no almanac author writes Rust.

Almanac's vision still says the compiler enforces the rules that matter
most. This repository restates principle 2 as "bundled in, checked at
build." The distance between those two sentences is exactly what the build
step has to close.
