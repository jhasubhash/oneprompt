# Security Policy

## Supported versions

oneprompt is in alpha. Only the latest release and the `main` branch receive security fixes.

## Reporting a vulnerability

Please do not open a public issue for a security problem. Report it privately through
GitHub instead:

1. Go to https://github.com/jhasubhash/oneprompt/security/advisories/new
2. Describe the problem, how to reproduce it, and which version you tested.

I aim to acknowledge a report within a week and to say what I plan to do about it soon
after. Once a fix is released, I am happy to credit you in the release notes if you wish.

## Scope

oneprompt is a terminal emulator, so the most relevant problems are those where
untrusted program output (escape sequences, pasted text, window title or clipboard
requests) can run commands, read files or leak data outside the terminal.
