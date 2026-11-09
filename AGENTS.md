# AGENTS.md

arbitrary-almanac is one Rust binary built on [Almanac](https://github.com/ivy/almanac), with workflows written in Fennel. The README holds the design, the layout, and the roadmap. Read it before changing anything.

## How to work here

- `just` is the gate. It runs `cargo fmt`, Clippy with warnings denied, and the tests, and it fails below 100% line coverage of the library. `just mutants` runs mutation testing. CI runs both, so a change is not done until both pass.
- Domain logic lives in the library under `src/`. The binary, `src/bin/arbitrary-almanac/main.rs`, only parses arguments, calls the library, and renders output.
- Library errors are variants of the `thiserror` and `miette::Diagnostic` enum in `src/error.rs`.

## Working with upstream Almanac

Almanac is ivy's repository, [ivy/almanac](https://github.com/ivy/almanac). This repository builds on Almanac's crates and changes them only upstream, never by copying or patching Almanac code here. The one planned exception is the Fennel host from ivy's spike ([ivy/almanac#5](https://github.com/ivy/almanac/pull/5)), which the roadmap promotes into `src/host/`.

Depend on Almanac's crates by git, on the `alpha/main` branch:

```toml
almanac = { git = "https://github.com/ivy/almanac", branch = "alpha/main" }
```

`alpha/main` is an integration branch: Almanac's `main` plus every `alpha/*` branch with an open pull request. It lets this repository build on upstream changes before they merge. It has no pull request of its own and never merges. `Cargo.lock` pins the commit it pointed at, so run `cargo update -p almanac` after it moves.

To change Almanac:

1. Make the change in the local checkout at `~/src/github.com/ivy/almanac`, a jj repository whose `origin` is ivy/almanac. Follow its AGENTS.md: `make check` is the gate there, and the repository is public, so its rules on private information apply.
2. Put the change on its own `alpha/<topic>` branch and open a pull request against ivy/almanac. If it depends on another open `alpha/*` branch, base it on that branch so the pull requests stack.
3. Rebuild `alpha/main` to include the new branch, and push it.
4. Run `cargo update -p almanac` here and commit the updated `Cargo.lock`.

With a single stack of pull requests, `alpha/main` is the tip of the stack. With several, it is a merge of their tips. Build it in the Almanac checkout:

```bash
# One stack: point at its tip.
jj bookmark set alpha/main -r <tip> --allow-backwards

# Several stacks: merge their tips.
jj new <tip> <tip>... -m 'alpha/main'
jj bookmark set alpha/main -r @ --allow-backwards

jj git push -b alpha/main
```

When upstream merges, rebases, or closes an `alpha/*` branch, rebuild `alpha/main` from the branches that are still open.
