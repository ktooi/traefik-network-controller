# Rust standard-library notices

`COPYRIGHT-library.html` is copied verbatim from the Rust **1.99.1** compiler distribution. It contains its license text inline; it has no relative license-file references. [ORIGIN.json](ORIGIN.json) records the upstream archive, compiler commit, path, and SHA-256.

This upstream document is a broad standard-library inventory. It includes platform-specific and build dependencies that need not be linked into this project's Linux GNU executable. Its presence does not mean every named component is shipped, or that any third-party component is relicensed under this project's MIT license.

The Docker image copies this directory to `/usr/share/licenses/traefik-network-controller/rust/`. Keep it with the executable when redistributing a binary outside that image. Consult [the licensing guide](../../docs/licensing.md) when updating the toolchain or publishing a different target.
