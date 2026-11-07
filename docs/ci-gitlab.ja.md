# GitLab CI/CD の設定

[English（正本）](ci-gitlab.md)。公開前に[ライセンス](licensing.ja.md)、実際の検証状況は [TESTING.md](../TESTING.md) を確認してください。

[`.gitlab-ci.yml`](../.gitlab-ci.yml) はソースを検証し、現在のプロジェクトの GitLab Container Registry に **Linux amd64** 用 Container image を公開します。認証には GitLab がジョブごとに発行する認証情報を使うため、Personal Access Token は不要です。このファイルは controller のデプロイや本番 Docker socket への接続を行いません。

## 1. 実行内容

| イベント | ジョブ | Registry への公開 |
| --- | --- | --- |
| Merge Request | `verify` | なし |
| 未オープン MR の feature branch への push | `verify` | なし |
| 保護された default branch への push | `verify` → `container_check` | なし |
| 保護された `vMAJOR.MINOR.PATCH` タグへの push | `verify` → `publish_container` | バージョン・履歴・ビルド・image の検証成功後 |
| その他のタグへの push | `verify` | なし |

`verify` は `bash scripts/ci/check.sh` を実行し、format、Clippy、Rust テスト全件、lockfile 固定の release build、CLI smoke test、Unix socket を使う mock Docker テスト、依存ライセンス通知の整合性を確認します。socket を使うテストの除外はありません。Runner はローカル Unix domain socket の bind を許可する必要があります。

Container 用ジョブは Dockerfile をビルドし、共通の [image smoke check](../scripts/ci/test_image.sh) を実行します。テストしたローカル image をそのまま push し、テスト後の再ビルドは行いません。この smoke check は、`TESTING.md` にある使い捨て Engine 上の lifecycle 検証や実際の経路を通る通信の検証を代替しません。

[GitLab の branch/MR 切り替えパターン](https://docs.gitlab.com/ci/yaml/workflow/)に沿って、feature branch の重複 pipeline を抑制します。default branch への push は常に独立した pipeline を実行します。このテンプレートでは schedule、`web`、API trigger、child pipeline の source は有効にしていません。MR の pipeline 画面から MR pipeline を再実行することは可能です。

## 2. GitLab.com プロジェクトの設定

1. `.gitlab-ci.yml`、`Cargo.lock`、`rust-toolchain.toml`、`scripts/`、テスト、ライセンスファイルを含むソース一式をプロジェクトへ配置します。CI/CD と Container Registry を有効にし、**Deploy → Container Registry** が利用できることを確認します。
2. GitLab-hosted Linux Runner を有効にし、アカウント／プロジェクトがジョブを実行できる状態にします。Runner tag の変数は両方とも `saas-linux-small-amd64` が既定値です。GitLab はこの Runner を使い捨て VM、Docker-in-Docker 対応と説明しています。[Runner の説明](https://docs.gitlab.com/ci/runners/hosted_runners/linux/)
3. default branch を保護し、merge の条件に pipeline 成功を設定します。CI ファイル、Dockerfile、lockfile、ライセンス検証ツール、release script の変更は、公開処理を実行するコードとしてレビューします。
4. **Settings → Repository → Protected tags** で `v*` を保護し、**Allowed to create** を release 担当者へ限定します。release tag を push する前に設定してください。Git tag の保護で公開用の参照を作れる人を制御し、YAML でも `CI_COMMIT_REF_PROTECTED` を確認します。[Protected tags](https://docs.gitlab.com/user/project/protected_tags/)
5. 実際の GitLab プロジェクトの **Build → Pipeline editor → Validate** で CI Lint を実行します。続いて branch の push または MR で `verify`、保護された default branch への merge で `container_check` を実行します。ローカルの YAML parse は GitLab の rules 評価や Runner での実行を代替しません。[CI Lint](https://docs.gitlab.com/ci/yaml/lint/)
6. image を一般公開するか決めます。匿名配布する場合はプロジェクトを public にし、Container Registry の visibility でも公開を許可します。最初の release 後に未認証の pull で確認してください。CI で認証済みの push が成功しても、匿名アクセスできるとは限りません。[Registry visibility](https://docs.gitlab.com/user/packages/container_registry/#change-visibility-of-the-container-registry)

この pipeline に Docker Hub や GitHub の認証情報は不要です。

## 3. Self-managed Runner

Linux x86-64 の Docker executor Runner を使用します。このテンプレートは shell executor 向けではありません。Runner tag が異なる場合は、次の秘密ではない project CI/CD variable を上書きします。

| 変数 | 既定値 | Self-managed の例 |
| --- | --- | --- |
| `TNC_VERIFY_RUNNER_TAG` | `saas-linux-small-amd64` | `tnc-verify-amd64` |
| `TNC_CONTAINER_RUNNER_TAG` | `saas-linux-small-amd64` | `tnc-container-amd64` |

検証用 Runner に privileged mode や Docker socket は不要です。Container 用 Runner は **privileged な Docker-in-Docker**、job と service で共有する `/certs/client` volume、および Registry、package repository、Rust 配布サーバー、Cargo 依存への到達性が必要です。job は使い捨ての service `tcp://docker:2376` に TLS 検証を有効にして接続します。Runner host の `/var/run/docker.sock` を mount したり、暗号化しない `2375` へ変更したりしないでください。[GitLab の TLS DinD 設定](https://docs.gitlab.com/ci/docker/docker_in_docker/)

専用の project Runner を作り、`tnc-container-amd64` tag、**Protected**、untagged job を実行しない設定にします。privileged job は隔離された使い捨て worker VM で実行し、本番 Traefik host および未信頼の MR 用 Runner から分離します。保護された default branch と release tag のジョブを受け付ける設定が必要です。YAML も Container 用ジョブをこれらの参照へ限定しています。[Runner protection](https://docs.gitlab.com/ci/runners/configure_runners/#prevent-runners-from-revealing-sensitive-information)・[Runner security](https://docs.gitlab.com/runner/security/)

GitLab が案内する方法で Runner を登録した後、既存 Runner stanza に以下のような設定を加えます。これは抜粋であり、登録設定の全体ではありません。生成された URL と認証 token は repository の外で保持してください。

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

長期間稼働する self-managed worker では、同時実行 job 間で固定の host 証明書 directory を共有しないでください。この例は Runner の同時実行数を 1 に制限しています。使い捨て worker は、認証情報や privileged job の状態が後続 job へ残ることも防ぎます。同じ worker の証明書保存先を別の Runner 登録と共有しないようにします。GitLab の DinD 文書も同時実行時の証明書 volume の競合を説明しています。独自 Registry CA を使う場合は job と DinD service の両方に信頼させ、TLS 検証は有効のままにします。

GitLab Self-Managed での Registry 有効化と Runner 容量の確保は管理者側の作業です。ガードを通すために GitLab の定義済み `CI_REGISTRY*`、`CI_COMMIT*`、pipeline source の変数を上書きしないでください。

## 4. 認証情報と信頼境界

| 設定 | 提供元 | 用途 |
| --- | --- | --- |
| `CI_REGISTRY` | GitLab の定義済み変数 | Registry endpoint |
| `CI_REGISTRY_IMAGE` | GitLab の定義済み変数 | 現在のプロジェクトの image repository |
| `CI_REGISTRY_USER` | GitLab の job ごとの認証情報 | `docker login` の username |
| `CI_REGISTRY_PASSWORD` | GitLab の job ごとの認証情報 | image テスト後に `--password-stdin` で渡す |

GitLab は job ごとに Registry の読み書きができる user を提供します。この pipeline 用に個人 password、PAT、本番 Docker の認証情報、恒久的な deploy token を追加する必要はありません。Docker client の認証ファイルは `after_script` で削除し、認証ファイルや証明書 directory を artifact／cache へ保存しません。[Registry authentication](https://docs.gitlab.com/user/packages/container_registry/authenticate_with_container_registry/)

検証／MR job に Registry login や publish の処理はありませんが、GitLab の CI job には `CI_JOB_TOKEN` が供給されます。login コマンドがないことは、悪意ある job code に対するセキュリティ境界にはなりません。fork の MR pipeline は fork 側で実行し、未レビューの fork code を親 project の認証情報が使える状態で手動実行しないでください。公開 CI へ無関係の secret も渡さないようにします。[Fork MR pipelines](https://docs.gitlab.com/ci/pipelines/merge_request_pipelines/#use-with-forked-projects)

## 5. Release の作成

まず対象バージョンを保護された default branch へ merge し、検証と Container check を成功させます。`TESTING.md` の該当する acceptance gate も完了させてください。CI 成功だけでは、実 Engine の network lifecycle や本番経路の通信を検証した証拠にはなりません。

レビュー済みの commit で working tree が clean であることを確認した後、現在のソースを初めて `0.2.0` として公開する場合の操作は次のとおりです。

```sh
bash scripts/ci/check.sh
python3 scripts/ci/release_metadata.py v0.2.0
git tag -a v0.2.0 -m "Release 0.2.0"
git push origin refs/tags/v0.2.0
```

この tag の作成・push は Registry への公開を意図して行う操作です。tag は厳密な `vMAJOR.MINOR.PATCH` 形式とし、prerelease suffix、build metadata、先頭の余分な `0` は許可しません。バージョンは `Cargo.toml` と一致する必要があります。publish job は現在の default branch を fetch し、tag の commit がその履歴に含まれることも要求します。未 merge の commit に付けた protected tag は Registry login 前に失敗します。

公開に成功すると次の両方が作成されます。

```text
$CI_REGISTRY_IMAGE:0.2.0
$CI_REGISTRY_IMAGE:sha-<full-commit-sha>
```

`latest`、major のみ、minor までの tag は書き込みません。`release-metadata.json` と `image-digests.json` は job artifact として 1 年間保持します。digest と pipeline の証拠を release 記録へ保存し、image を一意に固定する場合は、出力された `repository@sha256:...` 参照を使います。

`resource_group` は同じ Git tag の公開を直列化しますが、Registry tag を immutable にするものではありません。Registry が許可すれば、バージョン名／commit 名の Container tag はどちらも上書き可能です。base image tag の更新後に再ビルドすると、digest が変わる場合もあります。成功済みバージョンを再実行して中身を差し替えず、新しいバージョンを公開してください。GitLab edition が immutable container-tag rule に対応する場合は release tag に設定できますが、このテンプレートはそのオプション機能を必須としません。[Resource groups](https://docs.gitlab.com/ci/resource_groups/)・[Immutable container tags](https://docs.gitlab.com/user/packages/container_registry/immutable_container_tags/)

2 つの tag の push は一括トランザクションではありません。1 回目が成功し 2 回目が失敗した場合は、再試行前に Registry と job log を確認します。job の失敗だけでは何も公開されなかったとは判断できません。復旧のために公開済み Git tag を削除・付け替えしないでください。

## 6. 更新とトラブルシューティング

| 症状 | 確認点 |
| --- | --- |
| job が pending のまま | Runner tag、利用権限／保護設定、project Runner の有効化、空き容量 |
| `verify` が Unix socket 作成で失敗 | Runner sandbox の権限。テストは除外しない |
| `container_check` がない | MR／feature branch では仕様どおり。default branch への push なら保護設定を確認 |
| `publish_container` がない | tag push であること、tag 形式、Git tag の保護設定 |
| バージョンや履歴の検証で失敗 | Cargo version とレビュー済み default branch の履歴。ガードを迂回しない |
| Docker 接続／証明書で失敗 | `docker` service の起動、privileged mode、`/certs/client`、TLS 変数、Runner network policy |
| Registry で access denied | Registry の有効化、job の定義済み認証情報、project の権限、Registry 保護 rule |
| 匿名で image を pull できない | project／Registry visibility。CI の認証が制限を隠している可能性 |

Rust image のバージョンは `rust-toolchain.toml` と一致させ、Docker CLI と DinD は両方とも `29.9.3` tag に固定しています。公式 image metadata で tag の存在を確認していますが、バージョン tag 自体は変更可能です。image 更新をレビューし、ビルド入力を固定する release policy を採る場合は、実際に確認した Registry digest に固定してください。CLI と DinD のバージョンは対で更新します。[Rust 公式 image metadata](https://github.com/docker-library/official-images/blob/master/library/rust)・[Docker 公式 image tags](https://github.com/docker-library/docs/blob/master/docker/README.md)

この pipeline は source／revision／version／license の OCI metadata を記録します。image 署名、署名付き provenance、OS の完全な脆弱性評価、他 architecture のサポートを保証するものではありません。必要になった gate は明示的に追加してください。実際のプロジェクトでの初回 GitLab CI Lint、hosted／self-managed Runner の実行、Container build／smoke test、Registry への公開は実環境での確認が必要です。ファイルの準備完了は remote release 完了を意味しません。

**仕様の対応:** PUB-002（全件検証）、PUB-003（release／Registry 制御）、PUB-004 と DOC-006（設定手順・証拠）。外部資料の確認日: 2026-11-07 UTC。
