# Privacy boundary

## Scope

Hi Agent keeps the local copy of what the person supplied and removes PII, API keys,
passwords, and tokens immediately before a request leaves for an external language model.
Detection and masking are pure Rust code. HTTP is one consumer of secret references, not
part of detection.

The first implementation is text-only:

- `redact-core` provides maintained structured PII and secret detectors.
- `argus-redact-core` contributes maintained Chinese structured-ID patterns.
- There is no local NER or OCR.
- Hi Agent maintains policy and tests, not a private regex catalog.
- Detectors read a byte-for-byte ASCII stand-in of the text — every non-ASCII char's bytes
  replaced by newlines — so hit offsets still address the original. Their patterns are all
  ASCII-structured, while `regex`'s `\b` is Unicode-aware: read raw, `他的邮箱是alice@example.com`
  matches nothing, because `是` is a word char and there is no boundary before `alice`.

## Trust model

The trusted side is the local Hi Agent installation, including its drive and commands it
runs on the person's machine. The protected boundary is external model-provider transport.

| Zone | May hold or consume exact values |
|---|---:|
| Local input and raw journal | yes |
| Ordinary files in `drive/` | yes |
| Local commands and effectors | yes |
| Local owner UI | yes |
| External model request/context | no |
| Exported diagnostics and support bundles | no |

This is an egress-filtering design, not a vault sandbox. A model-generated command can
refer to and locally consume a secret file. The command should carry the path rather than
the credential characters and should not print the value.

The final request projector catches exact raw values in normal command output before a
later request reaches the model provider. It cannot provide a cryptographic guarantee
against arbitrary transformations performed by a command that can read the value, such as
encoding and intentionally exfiltrating it. Enforcing that stronger threat model would
require denying model-controlled commands direct access and using constrained brokers,
which is explicitly not the chosen design.

## Secret files

There is no hidden store, database, schema, or metadata record. Each retained credential is
one ordinary text file:

```text
drive/accounts/secrets/openai-api-key.txt
```

The path is the stable reference. Copying `drive/` copies both the credential and every
reference to it. The complete file content is the exact credential. There are no fields.
Secret files are written atomically and owner-only on Unix. They are plaintext at rest.

## Projection

Immediately before every serialized Responses request is sent upstream, the projector:

1. Replaces every known retained value by exact match.
2. Runs maintained PII and secret detectors over remaining text.
3. Resolves overlaps.
4. Replaces PII with typed masks and secrets with their drive-file reference.

Examples:

```text
alice@example.com
-> [PII:EMAIL_ADDRESS_1]

sk-proj-...
-> [SECRET_REF:drive/accounts/secrets/openai-api-key.txt]
```

Projection is recursive over JSON, so later model requests containing shell output, tool
results, object keys, or nested content cross the same boundary. A projection failure
blocks the provider request rather than failing open.

Known retained values are checked before detector heuristics. This remasks low-entropy
passwords and values whose format is not recognized on later turns.

### Transport

The projector's subject is the serialized request body, and nothing else. The local proxy
carrying that body upstream is otherwise transparent: every request header the agent
runtime sets reaches the provider verbatim, and every response header comes back the same
way. Only what the proxy is obliged to own is rewritten — the credential (the child holds
a per-boot proxy token, never the upstream key), the `Host` and `Content-Length` the hop
itself invalidates, the content coding the proxy terminates, and hop-by-hop headers.

A header allowlist is specifically wrong here. The runtime's transport metadata is ids and
feature flags it generated about its own turn, not person-supplied text, so it is out of
the projector's scope; and dropping any of it degrades the provider's view of a request
for no privacy gain, silently, in a way that shows up only as an upstream behaving oddly.

## Local use

### HTTP

`hi_http_request` accepts the drive path as `auth_ref`. The host reads the text file,
injects its content, follows no redirects, and projects the response.

```text
auth_ref = "drive/accounts/secrets/openai-api-key.txt"
```

### CLI

A model-generated command uses the file path and reads its content only at execution time:

```sh
OPENAI_API_KEY="$(cat drive/accounts/secrets/openai-api-key.txt)" some-cli
```

The exact invocation depends on the CLI's supported environment variables, stdin, config
files, or credential helpers. Prefer mechanisms that keep the value out of argv and avoid
commands that echo environment or config.

## Other model-visible reads

Journal ranges, session logs, and bounded UTF-8 file reads are projected by host readers.
The external model sees the same masks and drive-file references. Opaque media, PDF
extraction, archives, OCR, and NER are outside this boundary.

## Retention

Detected secrets are currently retained automatically. The target `this one` / `all` /
`none` preference and exchange-scoped temporary secret files are not implemented.

## Current limitations

1. Secret text files are plaintext at rest.
2. An arbitrary local command with secret-file access is outside a strict non-exfiltration
   threat model; the implemented guarantee is exact-value filtering at model egress.
3. Responses output/SSE and exported diagnostics still need a separate outbound projection
   pass for defense in depth.
4. Image, audio, and opaque file bytes are not inspected.
