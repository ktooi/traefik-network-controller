# Licensing and redistribution

English is canonical; [日本語](licensing.ja.md). Implements **PUB-001** in [SPEC.md](../SPEC.md).

## Decision

**MIT is an appropriate license for this project's original code and documentation.** It permits use, modification, redistribution and commercial use, provided its copyright and permission notice is retained. The repository now contains [LICENSE](../LICENSE), and `Cargo.toml` declares `license = "MIT"`. `publish = false` prevents an accidental crates.io upload; it does not prevent source or container distribution. See the [MIT text](https://opensource.org/license/mit) and [Cargo license field](https://doc.rust-lang.org/cargo/reference/manifest.html#the-license-and-license-file-fields).

The reviewed dependency versions have licensing paths that can accompany this MIT project while their own terms and attributions remain intact. **MIT does not relicense dependencies or the container's operating system.** Apache-2.0 components retain their Apache requirements, including applicable notices. Unicode and embedded BSD materials retain theirs. The license choices and complete collected notices are in [THIRD-PARTY-NOTICES.txt](../THIRD-PARTY-NOTICES.txt).

MIT is simple and suitable for this project's reuse goals. If an explicit contributor patent grant and patent-litigation termination clause becomes a requirement, Apache-2.0 contains those provisions in section 3; MIT has no corresponding express clause. There is no identified need to change the requested project license to Apache-2.0 merely because an Apache dependency is used. See [Apache-2.0 sections 3 and 4](https://www.apache.org/licenses/LICENSE-2.0).

The collective copyright line is `2026 Traefik Network Controller contributors`. It does not assign anyone's rights to a new legal entity. Contributors must have authority to submit their contributions under MIT; preserve third-party provenance when importing code. This review covers the files and dependencies described below, not an independent determination of every contributor's rights or a patent search.

## What was inspected

Review date: **2026-11-08 JST / 2026-11-07 UTC**. Source contract: **0.2.0**.

| Scope | Observed result |
| --- | --- |
| Repository source, examples, prompts and exported operator skill | No conflicting pre-existing license declaration was found in the inspected project files. The operator skill export is part of this project's original material. |
| Complete `Cargo.lock` / Cargo metadata | **98 external packages**, including non-Linux targets and build dependencies. Every locked package is covered by the generated inventory. |
| Linux GNU normal/build dependency graph | **84 external packages** for `x86_64-unknown-linux-gnu`. Build dependencies and unused portions of crates are not necessarily linked into the resulting executable. |
| Declared crate licenses | MIT, Apache-2.0, Unicode-3.0 and permissive alternatives, including BSL-1.0. MIT is selected where offered; `ryu` uses its BSL-1.0 option; `sync_wrapper` remains Apache-2.0. `unicode-ident` additionally requires Unicode-3.0. |
| Actual license files and embedded source notices | **278 materials**, represented by **83 distinct full texts**, are retained with package versions and locked crate checksums. |
| Rust standard library | The unmodified **Rust 1.99.1** `COPYRIGHT-library.html` is retained separately in [third_party/rust/](../third_party/rust/README.md), with version, compiler commit and file hash in `ORIGIN.json`. |
| Debian runtime image | Package licenses and source obligations remain separate. No final image was built or audited in this environment; see [TESTING.md](../TESTING.md). |

The review went beyond Cargo's package license strings. For example, `futures-channel` contains a BSD notice in `src/mpsc/queue.rs`; `tracing-core` includes another author's MIT notice under `src/spin/`; `icu_provider` contains a Rust-project attribution; and `tower` retains an Apache attribution for its optional Peak-EWMA implementation. These materials remain in the notices even when a particular feature or platform is not built. [BSD-2-Clause](https://spdx.org/licenses/BSD-2-Clause.html), [Unicode-3.0](https://www.unicode.org/license.txt) and Apache-2.0 require retaining applicable notices in their respective ways.

The Rust document is an upstream inventory broader than this executable, including platform-specific and build material. Its entries must not be treated as an exact list of linked components. Rust documents the purpose of `COPYRIGHT-library.html` in its [COPYRIGHT document](https://github.com/rust-lang/rust/blob/main/COPYRIGHT); use the bundled release-specific copy when redistributing this build.

## What to ship

| Distribution | Retained material |
| --- | --- |
| Project source archive | `LICENSE`, `THIRD-PARTY-NOTICES.txt`, `third_party/rust/`, `Cargo.lock`, and the corresponding source. |
| Controller binary outside the image | `LICENSE`, `THIRD-PARTY-NOTICES.txt`, and the matching `third_party/rust/` notices alongside the binary. Assess any system libraries separately for the chosen target. |
| Container image | Project and dependency notices under `/usr/share/licenses/traefik-network-controller/`, with Rust notices in its `rust/` directory. Preserve the base image's package copyright files and common licenses. |

The OCI `org.opencontainers.image.licenses=MIT` annotation identifies the controller project's license; it does not assert that every file in the image is MIT. Debian records package licenses in `/usr/share/doc/<package>/copyright`, with some common texts under `/usr/share/common-licenses/`. Do not remove those files as an image-size optimization. See [Debian copyright policy](https://www.debian.org/doc/debian-policy/ch-docs.html#copyright-information) and the [official Debian image's licensing guidance](https://hub.docker.com/_/debian).

Before publishing the first image, record its base digest and package inventory, check the retained copyright material in the **built image**, and arrange any corresponding-source distribution required by those packages' licenses. An upstream image tag or an SBOM alone does not discharge source-provision obligations. Keep this review tied to the actual image digest, especially when updating the Debian base or changing linkage/target. The Cargo notice check is not an operating-system license audit.

## Updating dependencies

Use Python **3.11 or newer** and the pinned Rust toolchain. After a dependency change, inspect new/changed license texts and nested attributions, then regenerate and verify:

```sh
cargo fetch --locked
python3 scripts/ci/check_licenses.py --write
python3 scripts/ci/check_licenses.py
```

`--write` updates only `THIRD-PARTY-NOTICES.txt`; it does not change the dependency lockfile or approve a new license. The ordinary check does not modify files. It rejects an unreviewed package license expression, an unexpected dependency source, an uncovered locked package, stale notices, or mismatched Rust notice metadata. The generator conservatively retains license/copyright/notice files and matching Rust/C source comment blocks; it is not a general license classifier or proof that a new dependency is legally suitable. Review the diff before accepting it. An isolated build with a populated cache can use `--offline`.

## Updating Rust

When changing the pinned toolchain, update the Docker builder to the same release and obtain that release through the normal verified Rust distribution process. Copy its **unmodified** `share/doc/rust/COPYRIGHT-library.html` from the selected toolchain into `third_party/rust/`; older releases may use `share/doc/rustc/`. Include any relative license files referenced by that document. Do not substitute the compiler's whole `COPYRIGHT.html` for the standard-library document or copy notices from an unrelated release.

Update `third_party/rust/ORIGIN.json` with the actual Rust release, full compiler commit, source archive URL/path, and SHA-256 for each retained file. Update the directory README and this guide's audit details. The check requires the selected compiler's release and commit to match this provenance, and checks every recorded file hash. A toolchain update is a fresh review; changing the version string alone is insufficient.

## Publication boundary

These files prepare the project for MIT distribution and keep notices reproducible. They do not publish a repository or image. Complete the actual CI/image checks and the runtime-image redistribution review before creating a public release tag. Preserve the distinction between license compatibility, vulnerability review and production acceptance: none of those three is proved by the others.
