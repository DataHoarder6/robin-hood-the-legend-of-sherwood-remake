# Releases and channels

Two channels, one tag each. A branch is never a channel.

| Channel | Source | Published as | Nix pin |
| --- | --- | --- | --- |
| `stable` | a `v*` tag | GitHub release, Velopack packages on the `win` and `linux` package channels, update feed = that tag | the tag's commit, `services.robin-hood.channel = "stable"` |
| `dev` | the nightly schedule (`native-release.yml`, `0 17 * * *` UTC) | rolling prerelease tagged `nightly-<date>`, version `<base>-nightly.<timestamp>+<sha>`, same package shape | a `fork/main` commit, `services.robin-hood.channel = "dev"` |

The daily run first compares the current commit with the last published
nightly and stops when they match, so an unchanged tree never replaces the
rolling release. `workflow_dispatch` takes an optional explicit version for a
release candidate.

The nix pin is always a commit, never a branch: a rebuild reproduces exactly the
build the launcher names, and the launcher prints the channel word beside the
short commit so two installs can be told apart. Nightly packages live in their
own release, so a stable install is never offered a nightly update and the
reverse holds too.

Known limit: this repository produces no workflow run for a push (see the map's
issue #26), so the nightly schedule plus a tag push are the only automated
entries, and a tag push is verified by a dispatch until the platform-side cause
is known.
