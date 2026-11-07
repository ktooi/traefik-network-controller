# プロンプト: リリースの準備または公開

Traefik Network Controller を `<GitHub/GHCR、GitHub と Docker Hub、または GitLab Registry>` 向けに準備してください。対象: `<確認済みのリポジトリ URL または checkout>`。安定版の予定: `<version>`。作業範囲: `<準備のみ、または明示的に許可したタグ作成／公開>`。`AGENTS.md`、`SPEC.md` の PUB-001–004／AC-19・AC-20、`docs/releasing.ja.md`、対象サービスの手順、`docs/licensing.ja.md`、実際の workflow、`TESTING.md` を読んでください。

1. 現在のソース、package／toolchain の版、ローカル変更を確認し、無関係な変更を保持します。実際のリポジトリ／既定ブランチ／registry を証拠から特定し、アカウント名や digest を作りません。
2. 確認済み依存関係と Rust の表示、ライセンス互換性、独自部分の権利を評価します。依存関係／toolchain が変わった場合は、再確認してから表示を再生成します。CI は確認結果を検査し、新しいライセンスを自動承認しません。
3. 必要な機能のある Linux で `bash scripts/ci/check.sh` を実行し、ソケットテストを残します。イメージの build／smoke は明示的に選んだ使い捨て CI Engine で実施します。合格・失敗・実行不可を分け、単体テストを実 Docker や通信の証拠にしません。
4. `python3 scripts/ci/release_metadata.py v<version>` で安定版タグを検査します。保護された既定ブランチのレビュー済みコードであることと、サービス側のタグ／environment／runner 制御を確認します。任意の Docker Hub は、結果確認も必要な追加公開先として扱います。
5. 認証情報はサービスの secret 管理を使い、手順に従って短期 token を利用します。secret の値を求めたり表示したりしません。未確認の fork workflow を信頼された親プロジェクトで実行しません。GitLab は login コマンドがないジョブにもサービス発行の token を渡します。
6. 許可済みのローカル準備を完成させ、変更、必要な設定、残る証拠を示します。既存の許可は維持しますが、準備の依頼だけでタグ作成／push、registry 公開、リポジトリ公開範囲変更、導入を許可済みとは扱いません。
7. 公開が明示的に許可され前提を満たす場合、新しい版タグを強制移動せず使い、パイプライン、各 registry の digest、公開範囲を確認します。YAML だけで push 成功とは言わず、復旧前に部分的な公開結果を調べて、公開済みの版を上書きしません。

ライセンス判断と限界、公開先、変更ファイル、SPEC 対応、実施検証と結果、必要なサービス設定を報告します。実在して確認できた場合だけ成果物／digest の URL を示します。公開だけで稼働中 controller は更新されず、導入には既存の許可と運用手順を適用します。
