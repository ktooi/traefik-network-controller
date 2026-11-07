# Release-matched task prompts / 用途別プロンプト

**Contract:** Traefik Network Controller 0.2.0, report schema 1. English files are canonical; `.ja.md` files provide the same workflow in Japanese.

Choose one prompt, supply its task inputs, and let the agent read the matching release's specification and command contract. A prompt does not install software, grant Docker permissions, or replace the installed binary's capabilities. Preserve the authorization already given in the session.

対象のプロンプトに入力を補い、対応するリリースの仕様と CLI 契約を読ませて使います。プロンプト自体はインストールや権限付与を行いません。既に与えた許可は維持し、導入済みバイナリの能力を確認します。

| Task / 目的 | English | 日本語 |
| --- | --- | --- |
| Evaluate / install / 導入 | [install.md](install.md) | [install.ja.md](install.ja.md) |
| Add application / アプリ追加 | [add-app.md](add-app.md) | [add-app.ja.md](add-app.ja.md) |
| Retire application / アプリ廃止 | [retire-app.md](retire-app.md) | [retire-app.ja.md](retire-app.ja.md) |
| Diagnose / 診断 | [diagnose.md](diagnose.md) | [diagnose.ja.md](diagnose.ja.md) |
| Implement change / 仕様駆動の実装 | [implement-change.md](implement-change.md) | [implement-change.ja.md](implement-change.ja.md) |
| Review change / 仕様駆動のレビュー | [review-change.md](review-change.md) | [review-change.ja.md](review-change.ja.md) |
| Prepare release / 公開準備 | [prepare-release.md](prepare-release.md) | [prepare-release.ja.md](prepare-release.ja.md) |

Operational tasks use [docs/operations.md](../docs/operations.md), [docs/cli.md](../docs/cli.md), and the distributed [operator skill](../skills/traefik-network-operator/SKILL.md). Development tasks additionally follow [AGENTS.md](../AGENTS.md) and [SPEC.md](../SPEC.md). Refer to [TESTING.md](../TESTING.md) for actual validation, not a presumed pass.
