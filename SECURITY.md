# Security policy

Cled reads your clipboard and sends it to your other devices. Clipboard content often includes
passwords, tokens, and private messages, so security problems in Cled can expose sensitive data.
We take reports seriously.

## Project status

Cled is in early, active development. It has **not** had an independent security audit. The
design aims to keep clipboard content private between paired devices (see
[RFC 0001](docs/rfcs/0001-lan-sync.md#7-security-summary)), but you should not treat it as
audited or production-ready. Release builds are not code-signed yet.

## Supported versions

Only the latest release and the `main` branch receive security fixes.

## Reporting a vulnerability

**Do not report security vulnerabilities in public GitHub issues, discussions, or pull
requests.**

1. Go to the repository's **Security** tab and choose **Report a vulnerability**. This uses
   GitHub's private vulnerability reporting, which only the maintainers can see.
2. If that option isn't available, open a public issue titled **"Security contact request"**
   that contains **no details** about the problem. A maintainer will arrange a private channel.

Please include:

- The affected version, commit, or build
- Your OS and version
- Steps to reproduce, or a proof of concept
- What an attacker could do (impact)

Don't include real clipboard content, keys, or other people's data in your report. Use test
data.

This is a volunteer project, so there is no guaranteed response time. We aim to acknowledge
reports promptly and will credit reporters in the fix unless they prefer not to be named.

## Scope

In scope:

- **Clipboard data:** content leaking to unpaired devices, to disk, to logs, or to the UI when it
  shouldn't; content marked private by password managers being read or synced.
- **Device pairing:** bypassing or brute-forcing the pairing code, pairing without user action,
  man-in-the-middle during pairing.
- **Authentication:** connecting or injecting items as a device that isn't paired, impersonating
  a paired device, removed devices keeping access.
- **Encryption:** weaknesses in how Cled uses Noise or SPAKE2, plaintext exposure of clipboard
  content on the network.
- **Network communication:** crashes, memory exhaustion, or denial of service from malformed or
  oversized network messages; information exposed through mDNS.
- **Local key storage:** exposure of the device's private key (`identity.key`) or tampering with
  the paired-devices file (`peers.json`).

Out of scope:

- Attacks that require control of the user's account or an unlocked device where Cled runs.
  Anyone with that access can read the clipboard directly.
- Warnings from unsigned builds (a known limitation, see [docs/building.md](docs/building.md)).
- Vulnerabilities in dependencies that don't affect Cled. Those are tracked automatically by
  the dependency audit in CI. Please report them upstream.
