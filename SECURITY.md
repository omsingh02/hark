# Security policy

## Supported versions

Security fixes are made on `main` and shipped in the latest release. Older releases are not patched.

## Reporting a vulnerability

Please do not report security problems in public issues, discussions or pull requests.

Report them privately through GitHub: open the [Security tab](https://github.com/omsingh02/hark/security), choose "Report a vulnerability", or go straight to <https://github.com/omsingh02/hark/security/advisories/new>.

Please include:

- the affected version (the output of `hark --version`) and your distribution,
- a description of the problem and its impact,
- steps to reproduce it, or a proof of concept.

The maintainer will acknowledge the report as soon as possible, keep you informed while a fix is prepared, and credit you in the release notes if you wish.

## Scope

hark is a user-level daemon. These are the parts that matter for security:

- It makes outbound HTTPS requests to a music recognition service and to a CDN for cover art, and it processes the responses.
- It exposes a D-Bus interface on the session bus: the standard MPRIS2 interfaces plus a few extra methods such as history queries. Any process running in the same user session can call it.
- It reads and writes files under `$XDG_RUNTIME_DIR/hark`, `~/.local/share/hark` and `~/.cache/hark`.

In scope are memory-safety bugs, unsafe handling of data received from the network (API responses and downloaded images), path traversal or unsafe file handling in the cache and history code, and abuse of the D-Bus interface that crosses a privilege boundary.

Out of scope are the behaviour or availability of the third-party recognition service, and attacks that already require control of the user's session or account.
