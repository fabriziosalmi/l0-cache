# Security Policy

We take the security of `l0-cache` seriously. If you believe you have found a security vulnerability, please report it to us privately so that we can address it before public disclosure.

## Reporting a Vulnerability

Please do **not** open a public GitHub issue for security-related bugs. 

Instead, report vulnerabilities privately by emailing the maintainer at:
**fabrizio.salmi@gmail.com**

Please include the following details in your report:
- A description of the vulnerability and its potential impact.
- Detailed steps to reproduce the issue (including any proof-of-concept scripts or commands).
- The environment and target platform where the issue was observed.

We will acknowledge receipt of your report within 48 hours and work with you to coordinate a security release.

## Supported Versions

Only the latest release version is supported and receives security patches. If you find a vulnerability, please verify if it exists in the latest release before reporting.

## Data Handling

- **Metrics file** — `l0-cache` writes one JSONL line per invocation to
  `$XDG_DATA_HOME/l0-cache/metrics.jsonl` (or `~/.local/share/...`), created with
  `0600` permissions (owner read/write only).
- **Command arguments** — the metrics `args` field is stored with obvious
  credentials redacted: `--password`/`--token`/`--secret`/`--api-key`/… values
  (both `--flag value` and `--flag=value`), `-H`/`--header` values, and URL
  userinfo (`scheme://user:pass@host`) become `***`. Redaction is best-effort and
  pattern-based; avoid passing secrets as positional arguments. Output sent to the
  terminal/LLM is also `$HOME`-redacted to `~`.

## Safety Guard scope

The optional [Safety Guard](README.md#safety-guard) is a **best-effort guard
rail, not a security boundary**. It pattern-matches argv and shell `-c` payloads
to block a few obviously destructive commands; it does not parse the full shell
grammar (quoting, substitution, variable expansion) and can be bypassed by a
determined caller. Do not rely on it to contain untrusted input.

## Installer

The `curl … | bash` installer clones the repository and builds from source with
`cargo`. It pins to the latest release tag rather than the branch tip. There are
no signed release binaries yet; if you require supply-chain guarantees, clone and
build a verified revision manually.
