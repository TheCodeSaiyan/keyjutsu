# Security policy

KeyJutsu runs commands on people's machines, so security reports get priority
over everything else.

## Reporting a vulnerability

Please do not open a public issue. Use GitHub's private vulnerability
reporting on this repository (the **Security** tab, then **Report a
vulnerability**).

Include what you can of: the version or commit, what you did, what happened,
what you expected, and whether it needs a malicious agent, a malicious
command, local access or nothing at all. A proof of concept helps; one that
changes a machine should target a disposable one.

You should get an acknowledgement within a week. Fixes for anything that lets
something run that the operator did not approve, escalate privilege, or leak
a secret come before feature work.

## What is in scope

Anything that breaks an invariant in [THREAT_MODEL.md](THREAT_MODEL.md). For
example: text reaching the shell that was not staged or typed by the operator;
submitting an incomplete staged command; defeating the disarm chord; forging a
completion mark without reading the session nonce; getting a proposal accepted
that carries KeyJutsu-owned fields.

Known limits listed in the threat model (such as the nonce being readable by a
process inside the session) are not new findings, but ideas for closing them
are welcome.

## Supported versions

KeyJutsu has not had a release yet. Until it does, fixes go to `main`.
