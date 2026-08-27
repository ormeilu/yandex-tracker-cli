# Install

The same binary is published three ways; pick whichever fits what you already
have installed.

## uv

No Rust toolchain needed. The wheel carries the compiled binary and no Python
code.

```bash
uvx yandex-tracker-cli --help      # run without installing
uv tool install yandex-tracker-cli # keep it around
```

## cargo

```bash
cargo install ytcli
```

## A binary

Download from [Releases](https://github.com/ormeilu/yandex-tracker-cli/releases)
— Linux, macOS and Windows, x86-64 and arm64 — and put it on your `PATH`.

## The command is `ytcli`

The PyPI package is named `yandex-tracker-cli`; the command it installs is
`ytcli`. A shorter name is not a cosmetic choice: an agent types it dozens of
times per session, and every one of those is tokens.

## Shell completions

```bash
ytcli completions zsh > ~/.zfunc/_ytcli
ytcli completions bash > /etc/bash_completion.d/ytcli
ytcli completions fish > ~/.config/fish/completions/ytcli.fish
```
