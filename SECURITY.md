# Security Policy

## Supported versions

Only the latest release gets security fixes.

## Reporting a vulnerability

Please do not open a public issue. Use [GitHub's private vulnerability reporting](https://github.com/yamamoto7/mobicat/security/advisories/new) instead, with steps to reproduce if you can.

You should get a reply within a week. Once a fix is released, the advisory is published with credit to you, unless you prefer otherwise.

mobicat runs `adb` and `xcrun` on your machine and reads their output. Reports about how it handles that output (for example, terminal escape sequences in log messages) are in scope.
