# GitLab CI/CD setup

English is canonical. [日本語](ci-gitlab.ja.md). See [licensing](licensing.md) before public distribution and [TESTING.md](../TESTING.md) for observed verification limits.

[`.gitlab-ci.yml`](../.gitlab-ci.yml) validates the source and publishes a **Linux amd64** container to the current project's GitLab Container Registry. It uses the project's per-job registry credentials; a personal access token is not required. This file does not deploy the controller or access a production Docker socket.

## 1. What runs

| Event | Jobs | Registry publication |
| --- | --- | --- |
| Merge request | `verify` | None |
| Feature-branch push without an open merge request | `verify` | None |
| Protected default-branch push | `verify`, then `container_check` | None |
| Protected `vMAJOR.MINOR.PATCH` tag push | `verify`, then `publish_container` | After version, ancestry, build and image checks succeed |
| Other tag push | `verify` | None |

`verify` invokes `bash scripts/ci/check.sh`: formatting, Clippy, the complete Rust test suite, locked release build, CLI smoke tests, Unix-socket mock Docker tests, and dependency-notice verification. No socket-dependent tests are skipped. The runner must permit a process to bind a local Unix-domain socket.

Container jobs build the Dockerfile and run the common [image smoke check](../scripts/ci/test_image.sh). The tested local image is the image pushed; there is no rebuild between smoke testing and publication. These smoke checks do not replace the disposable-Engine lifecycle or routed-traffic acceptance in `TESTING.md`.

The workflow uses [GitLab's branch/MR switching pattern](https://docs.gitlab.com/ci/yaml/workflow/) to avoid duplicate feature-branch pipelines. Pushes to the default branch always get their own pipeline. Scheduled, `web`, API-triggered and child pipeline sources are not enabled by this template; an MR pipeline can still be rerun from the MR's pipeline view.

## 2. Configure a GitLab.com project

1. Add the entire source tree to your project, including `.gitlab-ci.yml`, `Cargo.lock`, `rust-toolchain.toml`, `scripts/`, tests, and license files. Enable CI/CD and the Container Registry. Confirm that **Deploy → Container Registry** is available.
2. Enable the GitLab-hosted Linux runners and ensure your account/project can run jobs. Both runner-tag variables default to `saas-linux-small-amd64`; GitLab documents these runners as ephemeral VMs with Docker-in-Docker support. [Runner reference](https://docs.gitlab.com/ci/runners/hosted_runners/linux/)
3. Protect the default branch and require successful pipelines before merging. Review changes to CI files, the Dockerfile, dependency lockfile, license tooling and release scripts as executable release code.
4. In **Settings → Repository → Protected tags**, protect `v*`. Restrict **Allowed to create** to your release maintainers. Configure this before pushing a release tag. GitLab's protected Git tags control who may trigger the release reference; the YAML also checks `CI_COMMIT_REF_PROTECTED`. [Protected tags](https://docs.gitlab.com/user/project/protected_tags/)
5. Open **Build → Pipeline editor → Validate** and run CI Lint in your actual GitLab project. Then push a branch or open an MR to run `verify`; merge to the protected default branch to run `container_check`. Local YAML parsing cannot substitute for GitLab's rules evaluation or a runner execution. [CI Lint](https://docs.gitlab.com/ci/yaml/lint/)
6. Decide whether the image is public. For anonymous distribution, make the project public and configure its Container Registry visibility to allow public access. Confirm with an unauthenticated pull after the first release; a successful authenticated CI push does not prove anonymous access. [Registry visibility](https://docs.gitlab.com/user/packages/container_registry/#change-visibility-of-the-container-registry)

No Docker Hub or GitHub credentials are needed for this pipeline.

## 3. Self-managed runners

Use Linux x86-64 Docker executor runners. This template is not a shell-executor recipe. Override these non-secret project CI/CD variables if your runners have different tags:

| Variable | Default | Self-managed example |
| --- | --- | --- |
| `TNC_VERIFY_RUNNER_TAG` | `saas-linux-small-amd64` | `tnc-verify-amd64` |
| `TNC_CONTAINER_RUNNER_TAG` | `saas-linux-small-amd64` | `tnc-container-amd64` |

The verification runner needs no privileged mode or Docker socket. The container runner requires **privileged Docker-in-Docker**, a shared `/certs/client` volume for the job and service, and access to the registry, package repositories, Rust distribution servers and Cargo dependencies. The job connects to the disposable service at `tcp://docker:2376` with TLS verification enabled. Do not mount `/var/run/docker.sock` from the runner host and do not replace this with an unencrypted `2375` daemon. [GitLab's TLS DinD setup](https://docs.gitlab.com/ci/docker/docker_in_docker/)

Create a dedicated project runner, tag it `tnc-container-amd64`, mark it **Protected**, and disable untagged jobs. Use an isolated, disposable worker VM for privileged jobs, separate from the Traefik production host and from the untrusted MR runner. The protected runner must be eligible for the protected default branch and protected release tags. The YAML deliberately limits container jobs to those references. [Runner protection](https://docs.gitlab.com/ci/runners/configure_runners/#prevent-runners-from-revealing-sensitive-information) · [Runner security](https://docs.gitlab.com/runner/security/)

After registering the runner using GitLab's supplied registration instructions, the relevant existing runner stanza needs settings such as these. This is a fragment, not a complete registration file; retain the generated URL and authentication token outside the repository.

```toml
[[runners]]
  name = "tnc-container-amd64"
  executor = "docker"
  limit = 1
  [runners.docker]
    image = "docker:29.9.3-cli"
    privileged = true
    volumes = ["/certs/client"]
```

For a long-lived self-managed worker, do not share a fixed host certificate directory between concurrent jobs. The example limits that runner to one job; ephemeral workers also prevent credentials and privileged job state from carrying over to another job. Ensure no other runner registration shares that worker's certificate storage. The GitLab DinD documentation describes certificate-volume conflicts between concurrent jobs. Custom registry CAs must be trusted by both the job and DinD service; keep TLS verification enabled.

Registry enablement and runner capacity are administrator responsibilities on GitLab Self-Managed. Do not override GitLab's predefined `CI_REGISTRY*`, `CI_COMMIT*` or pipeline-source variables to force a job through its guards.

## 4. Credentials and trust boundary

| Setting | How it is supplied | Use |
| --- | --- | --- |
| `CI_REGISTRY` | GitLab predefined variable | Registry endpoint |
| `CI_REGISTRY_IMAGE` | GitLab predefined variable | Current project's image repository |
| `CI_REGISTRY_USER` | GitLab per-job credential | `docker login` username |
| `CI_REGISTRY_PASSWORD` | GitLab per-job credential | Passed through `--password-stdin` after image tests |

GitLab supplies a per-job registry user with read/write access. Do not add a personal password, PAT, production Docker credential or permanent deploy token for this pipeline. The Docker client authentication file is removed in `after_script`; neither it nor the certificate directory is an artifact or cache. [Registry authentication](https://docs.gitlab.com/user/packages/container_registry/authenticate_with_container_registry/)

The verification/MR job has no registry-login or publish step, but GitLab still supplies a `CI_JOB_TOKEN` to CI jobs. The absence of a login command is not a security boundary against malicious job code. Keep fork MR pipelines in the fork, do not manually run unreviewed fork code in the parent project's credential context, and do not expose unrelated secrets to public CI. [Fork MR pipelines](https://docs.gitlab.com/ci/pipelines/merge_request_pipelines/#use-with-forked-projects)

## 5. Create a release

First merge the intended version into the protected default branch and obtain successful verification and container checks. Complete the applicable acceptance gates in `TESTING.md`; CI success alone is not evidence that real network lifecycle or production traffic was accepted.

From the reviewed commit, after confirming the working tree is clean, a first release of the current source version would use:

```sh
bash scripts/ci/check.sh
python3 scripts/ci/release_metadata.py v0.2.0
git tag -a v0.2.0 -m "Release 0.2.0"
git push origin refs/tags/v0.2.0
```

Creating/pushing that tag intentionally requests registry publication. The tag must be exactly `vMAJOR.MINOR.PATCH`, with no prerelease suffix, build metadata or leading zeroes. Its version must equal `Cargo.toml`. The publish job fetches the current default branch and requires the tag's commit to be its ancestor; a protected tag on an unmerged commit fails before registry login.

Successful publication creates both:

```text
$CI_REGISTRY_IMAGE:0.2.0
$CI_REGISTRY_IMAGE:sha-<full-commit-sha>
```

No `latest`, major-only or minor-only tag is written. `release-metadata.json` and `image-digests.json` are retained as job artifacts for one year. Save the digest and pipeline evidence in the release record; deploy using the reported `repository@sha256:...` reference when you need exact image identity.

`resource_group` serializes publication for the same Git tag, but it does not make registry tags immutable. Both the version and commit-named container tags can be overwritten if the registry permits it, and rebuilding with updated base-image tags can produce a different digest. Do not rerun an already successful version to replace its contents; publish a new version. If your GitLab edition provides immutable container-tag rules, configure them for release tags; this template does not require that optional feature. [Resource groups](https://docs.gitlab.com/ci/resource_groups/) · [Immutable container tags](https://docs.gitlab.com/user/packages/container_registry/immutable_container_tags/)

Two tag pushes are not atomic. If the first succeeds and the second fails, inspect the registry and saved job log before retrying. A failed job does not prove that nothing was published. Do not delete or rewrite a published Git tag to recover.

## 6. Maintenance and troubleshooting

| Symptom | Check |
| --- | --- |
| Job remains pending | Runner tag, runner access/protection, project runner enablement and available capacity |
| `verify` fails creating a Unix socket | Runner sandbox permissions; keep the tests enabled |
| `container_check` is absent | This is expected on MR/feature branches; confirm default-branch protection for a default-branch push |
| `publish_container` is absent | Event is a tag push, tag format, and protected Git-tag rule |
| Tag-version or ancestry failure | Cargo version and reviewed default-branch history; do not bypass the guards |
| Docker connection/certificate failure | `docker` service startup, privileged mode, `/certs/client`, TLS variables and the runner's network policy |
| Registry access denied | Registry enablement, predefined job credential, project membership and registry protection rules |
| Image cannot be pulled anonymously | Project/registry visibility; CI authentication may have concealed the restriction |

The Rust image version matches `rust-toolchain.toml`; the Docker CLI and DinD images are pinned to matching `29.9.3` tags. Official image metadata confirms these tags, but version tags are still mutable. Review image updates and pin verified registry digests if your release policy requires fixed build inputs. Keep CLI/DinD versions paired. [Rust official image metadata](https://github.com/docker-library/official-images/blob/master/library/rust) · [Docker official image tags](https://github.com/docker-library/docs/blob/master/docker/README.md)

This pipeline records source/revision/version/license OCI metadata. It does not claim a signed image, signed provenance, a complete OS vulnerability assessment or support for other architectures. Add those gates deliberately when adopting them. First-run GitLab CI Lint, actual hosted/self-managed runner execution, container build/smoke and registry publication still need to be observed in your project; preparing these files is not a completed remote release.

**Specification mapping:** PUB-002 (full checks), PUB-003 (release/registry controls), PUB-004 and DOC-006 (setup and evidence). External documentation checked on 2026-11-07 UTC.
