# Install

The same binary is published three ways; pick whichever fits what you already
have installed.

## uv

No Rust toolchain needed. The wheel carries the compiled binary and no Python
code.

```bash
uvx --from yandex-tracker-cli ytcli --help   # run without installing
uv tool install yandex-tracker-cli # keep it around
```

## cargo

```bash
cargo install --git https://github.com/ormeilu/yandex-tracker-cli ytcli
```

Not on crates.io yet
([#16](https://github.com/ormeilu/yandex-tracker-cli/issues/16)). When it is,
the crate is `yandex-tracker-cli` and the command it installs is still `ytcli`
— `ytcli` as a package name belongs to somebody else on PyPI, and one name
across both registries is worth more than a short one.

## A binary

Download from [Releases](https://github.com/ormeilu/yandex-tracker-cli/releases)
— Linux, macOS and Windows, x86-64 and arm64 — and put it on your `PATH`.

## The command is `ytcli`

The package is named `yandex-tracker-cli` on both registries; the command it
installs is `ytcli`. A shorter name is not a cosmetic choice: an agent types it dozens of
times per session, and every one of those is tokens.

## Shell completions

```bash
ytcli completions zsh > ~/.zfunc/_ytcli
ytcli completions bash > /etc/bash_completion.d/ytcli
ytcli completions fish > ~/.config/fish/completions/ytcli.fish
```
