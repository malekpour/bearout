# Security policy

## Project status

Bearout is an experimental pre-release tool. It has not received a security
audit and is not ready to process repositories from untrusted authors.

What Bearout does provide is a capability-confined host with resource
limits:

- all filesystem access goes through a `cap-std` capability opened on the
  project root, and generated outputs are delivered only beneath the
  output roots declared in `bearout.toml`, never through symbolic links,
  and never over files Bearout does not own;
- repository policy runs in Starlark with no filesystem, environment,
  network, clock, or random access, with `load()` confined to the rules
  root, and under execution-tick, heap, and call-stack limits with
  cancellation;
- contract fixtures (`bearout test`) check virtual candidates through a
  read-only overlay that writes nothing, exposes nothing to policy, and
  runs declared formatters only under the same explicit authorization as
  `check`, with the suite bounded by its own limits;
- history checks (`bearout history`) read commit facts through the same
  hardened Git runner, from a policy pinned to the resolved head or the
  captured index, never fetch, write nothing, read no provider variable,
  and are bounded by their own limits. They do not verify signatures or
  the legal truth of a sign-off, and a commit policy is as trusted as the
  tree it is read from.

These measures bound accidents and runaway policy code. They are not a
sandbox: checking a repository written by a hostile author is not a
supported security boundary, and the limits are operational defaults, not a
proof of containment.

## Reporting

Report sensitive problems through GitHub's private vulnerability reporting
for this repository ("Report a vulnerability" under the Security tab), which
reaches the maintainer without creating a public issue. Do not put
sensitive details, reproduction steps, or affected data in a public issue.
If that channel is unavailable to you, open an issue stating only that you
need a private channel and the maintainer will arrange one.

Ordinary hardening suggestions and non-sensitive threat-model discussions
are welcome as public issues.
