# Running it on your own machine

This runs `build_failures` live against one GitHub repository from a
laptop or desktop, with no broker, ingress, or webhook. Almanac's daemon
runs two loops: one follows the deliveries log a broker writes, and one
runs a scheduled scan. With no broker the log stays empty, so every run
starts from a scan of the repository. A repair starts up to one scan
interval after a build fails.

Almanac's [deploying guide](https://github.com/ivy/almanac/blob/main/docs/deploying.md)
is the reference. This page lists only what differs.

## What you need

- Podman. On a Mac, a running `podman machine`.
- PostgreSQL, with a database: `createdb almanac`.
- The `claude` CLI, and a model credential: `ANTHROPIC_API_KEY` or
  `CLAUDE_CODE_OAUTH_TOKEN`.
- A repository on Renovate, with Almanac's `gitIgnoredAuthors` and
  `platformAutomerge: false` in its Renovate configuration, as the guide's
  step 4 says.

## 1. Build

In an Almanac checkout at `alpha/main`, build the fixer's sandbox image:

```bash
podman build -t localhost/almanac-fixer:latest images/polyglot
```

Here, build the binary and bundle the workflows into it:

```bash
cargo build --release
target/release/arbitrary-almanac build workflows -o ~/.local/bin/arbitrary-almanac
```

The broker is not needed.

## 2. Create the GitHub App

Follow the guide's step 2, with two changes: turn the App's webhook off,
and install it on the one repository. Grant no Workflows permission.

Use the App, not a personal token. A token acts as you, so Almanac could
not tell its own commits from yours, and it sees every repository you can.

## 3. Configure

Write an environment file from Almanac's
[`almanac-renovate.env.example`](https://github.com/ivy/almanac/blob/main/contrib/almanac-renovate.env.example),
readable only by you, and load it into the shell:

```bash
set -a; . ~/.config/almanac/arbitrary-almanac.env; set +a
```

`ALMANAC_RENOVATE_AUTHOR` is `renovate[bot]`, and `ALMANAC_BOT_LOGIN` is
the App's slug followed by `[bot]`.

## 4. Dry run

```bash
ALMANAC_READ_ONLY=1 arbitrary-almanac backfill --repo owner/repo --dry-run
```

**Check:** every red Renovate pull request is listed. A cooled one stops
at `an agent would run here`.

## 5. Go live

```bash
ALMANAC_MODE=live arbitrary-almanac run
```

**Check:** watch the log. On a red, cooled Renovate pull request,
`almanac/trust` goes `pending`, a commit lands on its branch, and the status
becomes `success` once CI passes.

**Stop:** Ctrl-C. An in-flight run gets 30 seconds, and anything unfinished
resumes on the next start.
