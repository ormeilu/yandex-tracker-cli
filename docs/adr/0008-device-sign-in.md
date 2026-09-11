# 8. Sign in through a shared application, with its secret in the binary

Date: 2026-09-10

## Status

Accepted

## Context

Getting a token used to mean registering an OAuth application, assembling an
authorize URL by hand, and copying the token out of the address bar. It was the
step where people gave up, and it had to be repeated whenever the token's scopes
changed — adding `wiki:read` for ADR 7, say.

Yandex OAuth supports the device-code flow: the tool asks for a short code, the
person confirms it at `oauth.yandex.ru/device` in any browser, and the tool,
polling, receives the token and a refresh token. It works over SSH and inside an
agent's sandbox, because the browser does not have to be on the same machine.

Exchanging the code, and later the refresh token, needs an application's id and
**secret**. The alternatives were a secret per user, which brings back the
registration step, and PKCE on an authorization-code flow, which drops the
secret but needs a fixed redirect registered in advance and a browser on the
same machine as the tool.

## Decision

One shared **ytcli** application, registered by the maintainer with the Tracker
and Wiki scopes. Its id and secret are compiled into release binaries from the
build environment (`YTCLI_OAUTH_CLIENT_ID`, `YTCLI_OAUTH_CLIENT_SECRET`) and never
committed. The same variables at run time take precedence, as a pair, so anyone
can sign in through an application of their own instead.

This is what `gh` does, and the reasoning is the same: the secret of a client
that runs on users' machines cannot be kept secret, and it does not need to be.
It identifies the application; it grants nothing on its own. Every token is still
issued by the person who signs in, on a Yandex page that names the application
and the scopes, and it can be revoked there.

`auth login` offers the browser first whenever the binary has an application.
Pasting a token stays — for CI, and for organisations that do not allow
third-party applications. `--read-only` narrows the request to `tracker:read wiki:read`
through the `scope` parameter.

The refresh token goes to the keychain next to the token, under its own service
name, keyed by account. A pasted token deletes it: it belongs to the grant the
paste replaced, and spending it later would bring that token back.

## Consequences

**The secret is extractable, and abuse would carry our name.** Someone could
build a phishing flow that shows "ytcli" on the consent screen. The exposure is
the same as every public CLI client's, and the remedy is too: the maintainer
rotates the secret, and a new release carries the new one.

**A build from source has no application** unless its environment supplies one.
It falls back to pasting, and says why rather than offering a sign-in that
cannot work.

**Refreshing is not rescoping.** `auth refresh` renews a token with the scopes it
already has. Changing them is another `auth login`, and the help says so,
because that is exactly the case people reach for refresh to solve.

ADR 1 and ADR 2 still hold as written: no token is taken as an argument or
printed, and both tokens live only in the keychain.
