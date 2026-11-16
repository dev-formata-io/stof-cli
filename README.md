<h1 align="center">
    <a href="https://stof.dev">
        <picture>
            <source height="110" media="(prefers-color-scheme: dark)" srcset="https://raw.githubusercontent.com/dev-formata-io/stof/main/content/stof.png">
            <source media="(prefers-color-scheme: light)" srcset="https://raw.githubusercontent.com/dev-formata-io/stof/main/content/image_dark.png">
            <img height="110" alt="Stof" src="https://raw.githubusercontent.com/dev-formata-io/stof/main/content/image_dark.png">
        </picture>
    </a>
    <br>
    <a href="https://crates.io/crates/stof-cli"><img src="https://img.shields.io/crates/v/stof-cli?label=stof-cli&color=aqua"></a>
    <a href="https://github.com/dev-formata-io/stof"><img src="https://img.shields.io/github/stars/dev-formata-io/stof"></a>
    <a href="https://stof.dev"><img src="https://img.shields.io/badge/docs-stof.dev-purple"></a>
    <a href="https://crates.io/crates/stof-cli"><img src="https://img.shields.io/crates/l/stof-cli?color=maroon"></a>
</h1>

<h3 align="center">The Stof command line: run, test, document, and package Stof documents.</h3>

[Stof](https://github.com/dev-formata-io/stof) is for software that behaves differently for every user: each customer's rules, tools, and state in one document that an agent can edit, a person can review, and your app runs, sandboxed, in Rust, JavaScript, and Python. This CLI is how you work with Stof documents directly: run them while you build, test them in CI (Ex. a customer's document before you approve an agent's change), generate docs, and bundle them into packages.

## Install

macOS and Linux:

```bash
curl --proto '=https' --tlsv1.2 -LsSf https://github.com/dev-formata-io/stof-cli/releases/latest/download/stof-cli-installer.sh | sh
```

Windows (PowerShell):

```powershell
powershell -ExecutionPolicy Bypass -c "irm https://github.com/dev-formata-io/stof-cli/releases/latest/download/stof-cli-installer.ps1 | iex"
```

These install the `stof` command, prebuilt, with no Rust needed. There are binaries for macOS (Apple silicon and Intel), Linux (x86_64 and ARM, static, so they run on any distribution), and Windows (x64). You can also download one from [Releases](https://github.com/dev-formata-io/stof-cli/releases) and put it on your `PATH`.

With Rust installed, `cargo binstall stof-cli` downloads the same binary, and `cargo install stof-cli` builds it from source.

To try Stof without installing anything, use the [Playground](https://stof.dev/playground).

## Quick start

`hello.stof`:

```rust
name: "world"

#[main]
fn main() {
    pln(`Hello, ${self.name}!`);
}

#[test]
fn has_a_name() {
    assert_eq(self.name, "world");
}
```

```text
$ stof run hello.stof
Hello, world!

$ stof test hello.stof
running 1 tests ...
test root has_a_name ... ok

test result: ok. 1 passed; 0 failed; finished in 0s
```

## Commands

| Command | What it does |
|---|---|
| `stof run [path]` | Runs every `#[main]` function. `-a <attr>` runs functions with another attribute instead (repeatable). |
| `stof test [path] [filter]` | Runs every `#[test]` function. The optional filter runs only tests on objects whose path contains it (Ex. `Libs.Http`). `--leaks` runs tests one at a time and fails any that leave objects behind (never dropped, not in any field). |
| `stof docs [path] [out]` | Writes Markdown docs for the document (and the standard library) into `out` (default `./`). `--tests` includes test functions. |
| `stof pkg [dir] [out]` | Bundles a directory with a `pkg.stof` file into a `.pkg` file (default `<dir>/out.pkg`). |
| `stof unpkg <file> [out]` | Unpacks a `.pkg` file into a directory (default `./stof/<name>`). |
| `stof lsp` | Starts the Stof language server on stdin/stdout, for editors (the [VS Code extension](https://marketplace.visualstudio.com/items?itemName=Formata.stof) uses it): errors and warnings, outline, go to definition, hover, completion, references, and rename. |

`path` can be a `.stof` file, any supported data file (JSON, YAML, TOML, ...), a package directory, or a `.pkg` file. It defaults to the current directory.

Global flags: `-d` shows `log_info` output, `-dd` also shows debug and trace logs. `stof <command> --help` lists every option.

### Running other attributes

Attributes are just labels, so one file can hold several entry points:

```rust
#[nightly]
fn cleanup() { pln("cleaning up"); }

#[weekly]
fn report() { pln("weekly report"); }
```

```bash
stof run jobs.stof -a nightly            # only #[nightly]
stof run jobs.stof -a nightly -a weekly  # both
```

### Errors and exit codes

Errors show the message, the line that failed, and the Stof call stack:

```text
$ stof run rates.stof
main root main ... failed
error: cannot multiply null: the right value is null or missing (use ?? to give a default)
  --> rates.stof:9:5
   |
 9 |     units * self.rate
   |     ^
  at root.total (rates.stof:9:5)
  at root.main (rates.stof:5:5)
```

Every command exits with code `1` when something fails (a parse error, a failing test, a failed `#[main]`), so `stof test` works as a CI step as-is.

## Packages

A package is a directory with a `pkg.stof` file that says what to import:

```rust
// my-lib/pkg.stof
name: "my-lib"
version: 0.1.0
import: ["src/main.stof"]   // files to import when the package is run or imported
exclude: ["^tests"]         // optional: regexes for paths to leave out of the .pkg
// include: ["^src"]        // optional: only package paths that match
```

```bash
stof run my-lib              # run the package directory
stof pkg my-lib              # -> my-lib/out.pkg
stof run my-lib/out.pkg      # run the packaged file
stof unpkg my-lib/out.pkg    # -> ./stof/out/
```

Inside Stof, `import pkg '@my-lib'` imports the package in `stof/my-lib` (where `stof unpkg` puts packages), and `import '@name'` imports the file `stof/name.stof`, both relative to the working directory.

## Permissions

Embedded Stof is sandboxed: a document can only see itself and the functions its host app gives it. The CLI runs your own files, so it turns on everything a script would expect: the file system (`fs`), environment variables (`env`), file and package imports, and the network (`Http::fetch`, including `localhost` and your private network). There's no memory limit, and `html` exports keep `style`, `action`, and `formaction`. Calls still have a time limit. Run files you'd trust as scripts.

When you embed Stof in an app, nothing is on until you allow it (`allow_system()`, `allow_http()`), and calls have time and memory limits. See [Security and the Sandbox](https://stof.dev/start/security).

## Learn more

- [Stof](https://github.com/dev-formata-io/stof): the runtime, embedding in Rust, JavaScript, and Python, and the changelog
- [stof.dev](https://stof.dev): docs and the standard library
- [Playground](https://stof.dev/playground): try Stof in the browser
- [Discord](https://discord.gg/Up5kxdeXZt): questions and discussion

## Building and releasing

`cargo build --release` builds against the published `stof` crate. To build against a local checkout of Stof (Ex. a change that isn't released yet), point Cargo at it for that build, and don't commit the `Cargo.lock` change it makes:

```bash
cargo build --release --config 'patch.crates-io.stof.path="../stof"'
```

Releases are built by [dist](https://axodotdev.github.io/cargo-dist/) (`dist-workspace.toml`, which generates `.github/workflows/release.yml`). To release:

1. Bump `version` in `Cargo.toml` (and the `stof` dependency, if it changed) and commit.
2. In GitHub, **Actions > Release > Run workflow**, with the tag `v<version>` (Ex. `v0.14.4`).

The workflow builds each platform, creates the GitHub Release with the binaries and installers, then publishes to crates.io with trusted publishing (the `release` environment, no token). It runs only when started by hand.

## License

Apache 2.0. See [LICENSE](LICENSE).
