# Japanese UI wording review (`apps/web/src/i18n/ja.ts`)

Review date: 2026-10-13. Ticked entries were applied to `ja.ts` on 2026-10-13, except the AI/LLM error messages (`errors.llm_*`, `errors.not_configured`, `activity.errorCodes.not_configured` and `activity.errorCodes.rate_limited`). The hackathon build has built-in model settings, so those messages are never shown.

How to read this file:

- **Priority**: **A** = wrong, misleading or clearly unnatural (fix this). **B** = reads like a translation or doesn't match
  what Japanese health apps say (recommended). **C** = polish / consistency (optional).
- "Current" is the text in `ja.ts` now. "Proposed" is a suggestion; tick the box when you accept it, or edit it.
- Keys are the paths inside the `ja` object, e.g. `today.tomorrowPlan.accept`.
- **Watch for knock-on edits:** the `guide.*` help texts quote button labels in 「」 (「プランを採用」「完了」「キャンセル」
  「承認」「栄養」 and so on). If you rename a button, also update every guide body that quotes it. Those places are marked ⚠︎ below.

## Reference apps and conventions used

| App / source | Wording taken from it |
|---|---|
| あすけん (asken) | 朝食・昼食・夕食・間食; 食事記録; 体重記録; 運動記録; **目標カロリー / 摂取カロリー / 消費カロリー**; 栄養素; **たんぱく質** (hiragana); PFCバランス; アドバイス; ふりかえり |
| カロミル | 「食事を記録」 (verb phrase on the button); PFCバランス; 記録はキーワード検索・画像・バーコード |
| MyFitnessPal (JP) | 炭水化物・脂質・たんぱく質; 目標; 食事の記録 |
| Fridge-stock / shopping apps (リミッター, かうサポ, しょくめも, 冷蔵庫ノート) | **在庫**; **賞味期限**; 期限切れ; **買い物リスト**; 買い忘れ; **購入済み**; 食材 |
| 厚生労働省「日本人の食事摂取基準」 | 身体活動レベル「低い・ふつう・高い」; エネルギー (formal term; consumer apps say カロリー) |
| Common JP web/app UX | 「しばらく時間をおいて、もう一度お試しください」; 「通信環境をご確認ください」; 「一定時間操作がなかったため、ログアウトしました」; avoid 「あなた」 in UI text; buttons are short verb phrases (〜する) |

Sources: [あすけん App Store](https://apps.apple.com/jp/app/id687287242), [あすけん紹介 (appllio)](https://appllio.com/app-asuken),
[カロミル App Store](https://apps.apple.com/JP/app/id963055562), [カロミル (app-liv)](https://app-liv.jp/963055562/),
[MyFitnessPal 解説 (libecity)](https://library.libecity.com/articles/01JQVCAD2T2RCPCATB9VV4MA7H),
[リミッター](https://appllio.com/app-limitter), [かうサポ](https://appllio.com/app-kausapo),
[かうサポ App Store](https://apps.apple.com/jp/app/id1234067923), [冷蔵庫ノート](https://aipicks.jp/showcase/reizoko-note)

---

## 0. Global issues (affect many strings)

| # | Pri | Issue | Proposal |
|---|---|---|---|
| G1 | A | **「あなた」 is overused** (activity, approvals, guide, members). Japanese UI text almost never says 「あなた」. It reads as translated and a little cold. | Drop it where the subject is obvious; use 「自分」 for filter/actor labels and 「ご自身」 in polite prose. See individual entries. |
| G2 | A | **Developer jargon in user-facing text**: モデル, トークン, ツール, 引数, API, モデル呼び出し. | Use 「AI」 for モデル, 「AI利用量」 for トークン (you can keep （トークン） in parentheses on the activity page), 「操作」 for ツール, 「詳細」 for 引数. |
| G3 | A | **Approve/reject = 「承認」「却下」** sound like a bureaucratic workflow. 却下 in particular is harsh for a personal app. | 「許可する」/「許可しない」 (or 「実行する」/「やめる」). Update all derived strings (approvals.*, activity.*, guide). ⚠︎ guide.steps[4], guide.pages.activity[3] |
| G4 | B | **Spacing between numbers and Japanese counters is inconsistent**: 「8 文字」「7 日間」「16〜70 歳」 vs 「16〜70歳」「20件」「1か月」. Native JP UI usually puts **no space** before 文字/日/回/歳/件/分/人. | Rule: no space before Japanese counters (7日間, 30分, 3回, 16〜70歳). For Latin units pick one style and keep it everywhere; あすけん writes `1,800kcal` with no space, but `500 kcal` is also acceptable. |
| G5 | B | **The `·` (U+00B7) middle dot separator** is a Western convention and renders oddly in JP fonts (`4 回 · 120 分`, `承認 3 · 却下 1`). | Use 「・」 between nouns, 「／」 or full-width space between figures, e.g. `4回／計120分`. |
| G6 | B | **「：」 used inside running sentences** (e.g. 「記録しています：チャット、…」). Fine in labels, unnatural in prose. | Rewrite as 「〜を記録しています（チャット、…）」 or split the sentence. |
| G7 | B | **「プログラムが計算します」** sounds like there's a separate program. JP apps say 「アプリが計算します」/「自動で計算します」. | Use 「アプリが自動で計算します」 everywhere (guide.steps[2], guide.pages.nutrition[0]). training.week.kcalHint already says アプリ. |
| G8 | B | **Retry phrasing is mixed**: 「再試行してください」「もう一度お試しください」「しばらくしてから再試行」. | Standardise on 「もう一度お試しください」 and 「しばらく時間をおいて、もう一度お試しください」. |
| G9 | B | **Same concept, different words**: 削除 vs 外す (members); 記憶 vs 保存 (preferences); 盛り付け vs 取り分け (portions); ケガ vs けが; 手で vs 手動で; 生年月 vs 生年月日; トレーニングの好み vs トレーニング設定; 自動の確認 vs アシスタントからの確認. | Pick one each: **削除**, **登録**, **取り分け量**, **けが**, **手動で**, **生まれた年月**, **トレーニング設定**, **アシスタントからの確認**. |
| G10 | C | **Number formatting**: kcal/token values are interpolated raw (`1800`, `523411`). JP apps show `1,800`. | Code change, not a string change: format with `toLocaleString("ja-JP")` in the template functions or the components. |
| G11 | A | **Errors tell the user to do something they can't do.** `errors.not_configured` says to set the model in settings, but Settings has no model section. `errors.llm_auth` mentions an API Key the user never sees. | See section 18. |

---

## 1. `common`

| ✓ | Pri | Key | Current | Proposed | Reason |
|---|---|---|---|---|---|
| [x] | A | `common.finishing` | 完了しています… | 処理中… | 「完了しています」 means "it is finished", not "finishing". |
| [x] | B | `common.retry` | 再試行 | もう一度試す | 再試行 is technical. Consumer apps use a verb phrase. |
| [x] | B | `common.networkError` | サーバーに接続できません。ネットワークを確認してから再試行してください。 | サーバーに接続できませんでした。通信環境をご確認のうえ、もう一度お試しください。 | 「通信環境をご確認ください」 is the standard JP phrasing (G8). |

## 2. `auth`

| ✓ | Pri | Key | Current | Proposed | Reason |
|---|---|---|---|---|---|
| [x] | B | `auth.usernameHint` | 半角英数字と _ - . で 3〜32 文字、大文字と小文字は区別しません | 3〜32文字の半角英数字・記号（_ - .）。大文字と小文字は区別しません | The current sentence is a run-on and the clauses are in an unnatural order. |
| [x] | C | `auth.passwordHint` | 8 文字以上 | 8文字以上 | G4 |
| [x] | C | `auth.register` | 登録 | 登録する | The submit button reads better as a verb (as on JP sign-up forms). |
| [x] | C | `auth.submitting` | お待ちください… | 処理中… | Shorter; matches the other loading labels. |
| [x] | C | `auth.toRegister` | アカウントをお持ちでない方は新規登録 | はじめての方は新規登録 | Shorter and friendlier; very common JP pattern. Optional. |
| [x] | B | `auth.sessionExpired` | ログインの有効期限が切れました。もう一度ログインしてください。 | 一定時間が経過したため、ログアウトしました。もう一度ログインしてください。 | Standard JP session-timeout wording. |

## 3. `nav`

| ✓ | Pri | Key | Current | Proposed | Reason |
|---|---|---|---|---|---|
| [x] | B | `nav.nutrition` | 栄養 | 食事 | The page shows meals, weight and the meal plan. JP apps (あすけん, カロミル) call this area 「食事」/「食事記録」. ⚠︎ guide.steps[3] title/body and `overview.noDietPlan` quote 「栄養」. |
| [x] | C | `nav.activity` | アシスタント履歴 | AIの操作履歴 | Says more clearly what the page holds. Optional; if you change it, also update `settings.features.activityTab` and guide.steps[4]. |
| [ ] | C | `nav.stock` | 在庫 | 食材ストック | 在庫 is OK (stock apps use it), but on its own it sounds like a warehouse. Optional. |

## 4. `guide`

| ✓ | Pri | Key | Current | Proposed | Reason |
|---|---|---|---|---|---|
| [x] | A | `guide.gotIt` | わかった | OK | Too casual for a button; the rest of the UI is です/ます. |
| [x] | B | `guide.steps[0].body` | HHH は食事とトレーニングを記録し、あなたの目標に合わせて毎日翌日のプランを作ります。基本的な使い方を簡単にご紹介します。 | HHH は食事とトレーニングを記録し、目標に合わせて翌日のプランを毎日自動で作ります。基本的な使い方をご紹介します。 | 「毎日翌日の」 is awkward; drop あなた (G1); 簡単に＋ご紹介 is redundant. |
| [x] | B | `guide.steps[1].title` | 今日：今日をひと目で | 「今日」画面：1日の予定をひと目で | 「今日：今日を」 repeats itself. |
| [x] | B | `guide.steps[2].body` | …カロリーと栄養素はプログラムが計算します。… | …カロリーと栄養素はアプリが自動で計算します。… | G7 |
| [x] | B | `guide.steps[3].title` | 機能画面：栄養・トレーニング・メンバー・在庫 | 各機能の画面：食事・トレーニング・メンバー・在庫 | 「機能画面」 isn't a natural noun; follows the nav rename. |
| [x] | B | `guide.steps[4].body` | …大事な操作は、あなたが「承認」するまで実行されません。 | …大事な操作は、「許可する」を押すまで実行されません。 | G1, G3 |
| [x] | C | `guide.steps[5].body` | …アシスタントは選んだ言語で返事をします。… | …アシスタントも選んだ言語で返信します。… | 返事 is casual; 返信 is the usual chat-app word. |
| [x] | B | `guide.pages.nutrition[0].body` | …目標はプロフィール・体重・目標からプログラムが計算します。…「推定」の印は… | …目標値は、プロフィール・体重・目標をもとにアプリが自動で計算します。…「推定」マークが付いた値はアシスタントが推定した栄養値です。… | G7. 「目標は…目標から」 repeats itself. 「印」 → 「マーク」 is more natural in UI. |
| [x] | B | `guide.pages.nutrition[1].body` | …カロリー目標は最新の体重で更新されるので、定期的な記録がおすすめです。 | …目標カロリーは最新の体重をもとに更新されるため、定期的に記録しましょう。 | あすけん says 「目標カロリー」, not 「カロリー目標」. |
| [x] | B | `guide.pages.nutrition[2].body` | …記録済みの食事には「食べた」と表示されます。 | …記録済みの食事には「記録済み」と表示されます。 | Follows `today.todayPlan.eaten`. |
| [x] | B | `guide.pages.nutrition[3].body` | …納得したら「プランを採用」を押します。 | …内容がよければ「このプランにする」を押します。 | Follows `today.tomorrowPlan.accept`. 「納得したら」 sounds like you need to be persuaded. |
| [x] | B | `guide.pages.nutrition[4].title` | 注意点と一緒に食べる食事 | 注意事項と、みんなで食べる食事 | As written it can parse as "food eaten together with notes". |
| [x] | B | `guide.pages.nutrition[4].body` | …一緒に食べる食事には同席者と合計人数分が表示されます。… | …一緒に食べる食事には、一緒に食べる人と合計の人数が表示されます。… | 「同席者」 is business-meeting language; 「合計人数分が表示」 is ungrammatical. |
| [x] | B | `guide.pages.training[2].body` | 明日どう動きたいか（例：…）を書いて保存し、… | 明日どんなトレーニングをしたいか（例：…）を入力して保存し、… | 「どう動きたいか」 is vague. |
| [x] | B | `guide.pages.training[3].title` | トレーニングの好み | トレーニング設定 | G9. Matches `toolSummaries.training_preferences_updated`. |
| [x] | B | `guide.pages.training[3].body` | …ケガなどのメモを設定します。アシスタントはプランを作るときにこれらを守ります。 | …けがなどのメモを設定します。アシスタントはこれらをもとにプランを作ります。 | 「守ります」 is a literal translation of "respects". G9 (けが). |
| [x] | B | `guide.pages.members[1].body` | …変わったら手で更新してください。… | …変わったら手動で更新してください。… | G9 |
| [x] | C | `guide.pages.members[2].body` | …（月〜日の朝・昼・夕）… | …（月〜日の朝・昼・夜）… | 朝・昼・夜 is the everyday set; 夕 on its own is uncommon. |
| [x] | B | `guide.pages.activity[0].body` | アシスタントがあなたの代わりにしたことをすべて記録しています：チャット、プランの定時作成、プランの調整、操作の承認・却下。何をしたのかをいつでも確かめられます。 | アシスタントが行った操作をすべて記録しています（チャット、プランの自動作成・調整、操作の許可など）。いつでも内容を確認できます。 | G1, G3, G6 |
| [x] | B | `guide.pages.activity[1].body` | …確認リクエストの承認・却下数、安全ブロックの回数、モデルのトークン使用量を表示します。 | …確認の許可・不許可の数、安全のために止めた操作の数、AIの利用量（トークン）を表示します。 | G2, G3 |
| [x] | B | `guide.pages.activity[2].body` | 実行者（あなた／自動）と状態… 「ステップを表示」で、呼び出したモデルやツールとその結果が見られます。 | 実行者（自分／自動）と状態… 「詳細を表示」で、AIが行った処理とその結果を確認できます。 | G1, G2. Follows `activity.showSteps`. |
| [x] | B | `guide.pages.activity[3].body` | …ここで承認・却下できます。… | …ここで許可・不許可を選べます。… | G3 |
| [x] | B | `guide.pages.stock[1].title` | 2 つのスイッチ | 2つの設定 | 「スイッチ」 is a literal rendering of "toggle". |
| [x] | B | `guide.pages.stock[2].body` | …数量はその場で直せ、… | …数量はその場で変更でき、… | 「直せ」 is too colloquial for help text. |
| [x] | A | `guide.pages.stock[3].body` | 明日のプランに必要な食材を一覧にし、在庫にある分も表示します。買ったら「完了」でプランの量が在庫に入ります。買わないものは「キャンセル」、「元に戻す」で取り消せます。 | 明日のプランに必要な食材を一覧にし、在庫にある分も表示します。購入したら「買った」を押すと、プランの分量が在庫に追加されます。買わないものは「買わない」を押します。押し間違えたら「元に戻す」で取り消せます。 | The last sentence is confusing: it reads as though 「キャンセル」 and 「元に戻す」 do the same thing. Follows the `stock.*` button renames. |
| [x] | B | `guide.pages.stock[4].title` | 最近使った分 | 最近使った食材 | 「使った分」 is vague as a heading. |

## 5. `onboarding`

| ✓ | Pri | Key | Current | Proposed | Reason |
|---|---|---|---|---|---|
| [x] | B | `onboarding.profile.birthDate` (also `members` form) | 生年月 | 生まれた年月 | 「生年月」 is rarely used (people know 生年月日). Errors also say 生年月日, which is wrong because only year and month are collected (see §18). |
| [x] | B | `onboarding.profile.activityLevelHints.*` | 座りがち — ほとんど運動せず、主にデスクワーク / 軽い活動 — … / 中程度の活動 — … / 活発 — … / 非常に活発 — … | ほとんど運動しない（デスクワーク中心）/ 軽い運動をする（週1〜3日）/ 適度に運動する（週3〜5日）/ よく運動する（週6〜7日）/ 非常によく運動する（激しいトレーニング・肉体労働） | The em dash format is English style. 「座りがち」「活発」 are translations of the Western PAL labels; describing the behaviour is clearer. |
| [x] | C | `onboarding.profile.dietaryPatternOptions.pescatarian` | ペスカタリアン | ペスカタリアン（魚介は食べる） | Not a well-known term in Japan. Adding a gloss helps. |
| [x] | C | `onboarding.profile.dietaryPatternOptions.vegan` | ヴィーガン | ヴィーガン（動物性食品をとらない） | Same reason. Optional. |
| [x] | B | `onboarding.profile.allergiesPlaceholder` / `dislikesPlaceholder` | 読点で区切って入力（例：えび、ピーナッツ） | 「、」で区切って入力（例：えび、落花生） | 「読点」 is grammar-class vocabulary. 落花生 is the legal allergen name in JP labelling. Optional. |
| [x] | B | `onboarding.profile.medicalNotes` | 健康状態のメモ | 持病・健康上の注意 | Clearer about what to write. |
| [x] | B | `onboarding.profile.medicalNotesPlaceholder` | 関係する健康状態があればご記入ください（任意） | 例：高血圧で塩分を控えている、糖尿病で通院中 | An example helps more than an instruction, and 「任意」 is already shown by the label. |
| [x] | B | `onboarding.goal.direction` | 目標の方向 | 目的 | 「目標の方向」 is a literal translation of "direction". |
| [ ] | C | `onboarding.goal.directionOptions` | 減量 / 維持 / 増量 | 体重を減らす / 体重を維持する / 体重を増やす | Optional. 減量/維持/増量 is acceptable and shorter. |
| [x] | B | `onboarding.goal.ratePerWeek` | 1 週間あたりの変化量 | 1週間あたりのペース | 「変化量」 sounds like a math term. Also used in `errors.goal_rate_out_of_range`. |
| [x] | B | `onboarding.goal.previewTitle` | 1 日の目標カロリー（見込み） | 1日の目標カロリー（目安） | 「目安」 is the usual word. |
| [x] | B | `onboarding.goal.previewFloorApplied` | 安全な下限値に合わせて調整しました。 | 健康を損なわないよう、下限値まで引き上げています。 | Says why the number changed. |
| [x] | B | `onboarding.goal.previewError` | 現在プレビューを計算できません。上の入力内容を確認してください。 | 目標カロリーを計算できません。入力内容をご確認ください。 | 「プレビュー」 is internal wording. |
| [x] | C | `onboarding.issues.title` | 次に進むには： | 次に進むには、以下を入力してください | Complete sentence. |

## 6. `today`

| ✓ | Pri | Key | Current | Proposed | Reason |
|---|---|---|---|---|---|
| [x] | C | `today.diet.noTarget` | 目標が未設定です。プロフィールと体重を登録してください | 目標が未設定です。プロフィールと体重を登録してください。 | Missing 「。」. |
| [x] | A | `today.todayPlan.eaten` | 食べた | 記録済み | Casual plain form used as a status badge. |
| [x] | A | `today.todayPlan.notYetEaten` | まだ | 未記録 | 「まだ」 alone as a badge is unclear. |
| [x] | B | `today.tomorrowPlan.accept` | プランを採用 | このプランにする | 「採用」 reads like hiring or adopting a proposal. ⚠︎ guide.pages.nutrition[3] |
| [x] | C | `today.tomorrowPlan.adjust` | 調整 | 調整する | Pairs with 「作り直す」 (both verbs). ⚠︎ guide |
| [x] | B | `today.tomorrowPlan.warningsTitle` | 注意 | 注意事項 | 「注意」 alone looks like an alert icon label. |
| [x] | B | `today.tomorrowPlan.medicalDisclaimer` | …このプランは生活習慣の提案にすぎません。医師や管理栄養士の指示を優先してください。 | …このプランは一般的な食生活のアドバイスです。治療中の方は、医師や管理栄養士の指示を優先してください。 | 「〜にすぎません」 sounds dismissive. |
| [x] | C | `today.tomorrowPlan.trainingKcalAdded` | トレーニング分として ${kcal} kcal を追加 | トレーニング分 +${kcal} kcal | Shorter. |
| [ ] | C | `today.tomorrowPlan.requestLabel` | 今回のプランへの要望 | プランへのリクエスト | Optional. |

## 7. `overview`

| ✓ | Pri | Key | Current | Proposed | Reason |
|---|---|---|---|---|---|
| [x] | B | `overview.noDietPlan` | …「栄養」画面で明日のプランを作れます。 | …「食事」画面で明日のプランを作成できます。 | Follows the nav rename. |

## 8. `stock`

| ✓ | Pri | Key | Current | Proposed | Reason |
|---|---|---|---|---|---|
| [x] | C | `stock.empty` | 在庫はありません | 登録された食材はありません | 「在庫はありません」 sounds like a shop saying it's "out of stock". |
| [ ] | B | `stock.expiresOn` | 賞味期限 | 期限（賞味・消費） | Fresh food has 消費期限, not 賞味期限. Optional; fridge apps often just say 賞味期限. |
| [x] | C | `stock.shoppingFor` | ${d} のプランより | ${d} のプラン用 | Reads more naturally. |
| [x] | C | `stock.nothingToBuy` | 在庫で足りるので、買う物はありません。 | 在庫で足りるため、買い足すものはありません。 | |
| [x] | A | `stock.complete` | 完了 | 買った | 「完了」 is ambiguous on a shopping list. JP list apps use 購入済み or a check mark. ⚠︎ guide.pages.stock[3] |
| [x] | B | `stock.completeHint` | 買えた：プランの量を在庫に追加します | 購入済みにして、プランの分量を在庫に追加します | 「買えた」 is too casual. |
| [x] | A | `stock.cancel` | キャンセル | 買わない | 「キャンセル」 sounds like it cancels an order or closes a dialog. ⚠︎ guide.pages.stock[3] |
| [x] | B | `stock.statusCancelled` | キャンセル済み | 買わない | Follows the button. |
| [x] | B | `stock.recentTitle` | 最近使った分 | 最近使った食材 | |

## 9. `training`

| ✓ | Pri | Key | Current | Proposed | Reason |
|---|---|---|---|---|---|
| [x] | C | `training.week.summary` | ${sessions} 回 · ${minutes} 分 | ${sessions}回・計${minutes}分 | G4, G5 |
| [x] | B | `training.week.kcal` | 運動消費 約 ${kcal} kcal | 消費カロリー 約${kcal} kcal | 「消費カロリー」 is the standard term (あすけん). |
| [x] | B | `training.preferences.title` | トレーニングの好み | トレーニング設定 | G9 |
| [x] | B | `training.preferences.goalOptions` | 総合的な体力 / 筋力 / 筋肥大 / 持久力 / 減量 | 体力づくり / 筋力アップ / 筋肉をつける / 持久力アップ / 脂肪を落とす | These are goals, so they should read as goals. 「総合的な体力」 and 「筋肥大」 sound like textbook terms. |
| [x] | C | `training.preferences.daysPerWeek` | 週のトレーニング日数 | 週あたりの日数 | Shorter; the section title already says トレーニング. |
| [x] | C | `training.tomorrow.preferenceLabel` | 明日はどんなトレーニングにしたいですか？（任意） | 明日やりたいトレーニング（任意） | Optional. A form label usually isn't a question. |

## 10. `chat`

| ✓ | Pri | Key | Current | Proposed | Reason |
|---|---|---|---|---|---|
| [x] | B | `chat.readOnly` | 質問のみ（記録を変更しない） | 相談のみ（記録しない） | Shorter, and 「相談」 covers asking for explanations or calculations. ⚠︎ guide.steps[2], guide.pages.activity[3], `activity.toolOutcomes.read_only_mode` |
| [x] | A | `chat.readOnlyHint` | 翻訳・説明・計算向け。健康情報、食事記録、プランは変更しません。会話は保存されます。 | 質問や相談だけしたいときに使います。食事記録・プラン・健康情報は変更されません（会話は保存されます）。 | 「翻訳・説明・計算向け」 is a fragment that reads like a spec note. |
| [x] | C | `chat.empty` | まだ会話がありません。何か話しかけてみましょう | 食べたものや運動したことを話しかけてみましょう（例：「お昼にラーメンを食べた」） | An example makes the first use easier. |
| [x] | C | `chat.sendFailed` | 送信に失敗しました | 送信できませんでした | Softer; common JP phrasing. |
| [x] | B | `chat.notices.empty_reply` | モデルから返答がありませんでした。… | アシスタントから返答がありませんでした。… | G2 |
| [x] | B | `chat.notices.too_many_steps` | 処理の手順が多すぎたため停止しました。完了した操作は下のとおりです。残りはもう一度送ってください。 | 処理が多すぎたため途中で止めました。完了した内容は下に表示しています。残りはもう一度送信してください。 | Smoother. |

## 11. `approvals`

| ✓ | Pri | Key | Current | Proposed | Reason |
|---|---|---|---|---|---|
| [x] | B | `approvals.hint` | アシスタントが次の変更を求めています。承認するまで実行されません。 | アシスタントが次の変更を行おうとしています。「許可する」を押すまで実行されません。 | 「変更を求めています」 sounds like the AI is making a demand. G3 |
| [x] | A | `approvals.approve` | 承認 | 許可する | G3 |
| [x] | A | `approvals.reject` | 却下 | 許可しない | G3 |
| [x] | B | `approvals.approved` | 承認して実行しました | 実行しました | |
| [x] | B | `approvals.rejected` | 却下しました（変更なし） | 取りやめました（変更はありません） | |
| [x] | C | `approvals.expired` | 期限切れです（変更なし） | 期限切れです（変更はありません） | |
| [x] | B | `approvals.failed` | 承認しましたが、実行に失敗しました：${reason} | 実行できませんでした：${reason} | |
| [x] | B | `approvals.previews.add_preference` | …を保存：…（ずっと）/（1か月） | …を登録：…（期限なし）/（1か月間） | 「ずっと」 is too casual; `settings.preferences.permanent` already says 期限なし. G9 |
| [x] | C | `approvals.previews.remove_preference` | …を削除：… | (same) | OK |
| [x] | C | `approvals.previews.update_member_allergies` | ${d.name} さんの… | ${d.name}さんの… | Inconsistent さん spacing (elsewhere `${d.guest}さん`). |
| [x] | C | `approvals.previews.update_training_notes` | トレーニングのメモ（けが等）を変更… | トレーニングのメモ（けがなど）を変更… | 「等」 is written-document style. |
| [x] | B | `approvals.previews.add_food_alias` | 「${alias}」を「${food}」として記録 | 「${alias}」を「${food}」と同じ食品として扱う | 「として記録」 sounds like a meal is being logged. This also pairs with `remove_food_alias`. |

## 12. `activity`

| ✓ | Pri | Key | Current | Proposed | Reason |
|---|---|---|---|---|---|
| [x] | A | `activity.hint` | アシスタントがあなたの代わりに行ったことをすべて記録しています。誰が始めたか、何を呼び出したか、何が止められ、何があなたの確認待ちかが分かります。 | アシスタントが行った操作をすべて記録しています。きっかけ、実行した処理、止められた操作、確認待ちの操作を確認できます。 | G1. The parallel list breaks grammar (〜か、〜か、〜られ、〜か). |
| [x] | B | `activity.stats.byUser` | あなたが開始 | 手動 | Pairs with 「自動実行」. G1 |
| [x] | B | `activity.stats.approvals` | 確認リクエスト | 確認 | |
| [x] | B | `activity.stats.approvalsDetail` | 承認 ${a} · 却下 ${r} | 許可 ${a}／不許可 ${r} | G3, G5 |
| [x] | B | `activity.stats.guardrails` | 安全ブロック | ブロックした操作 | 「安全ブロック」 isn't a word. |
| [x] | B | `activity.stats.tokens` | モデルのトークン | AI利用量（トークン） | G2 |
| [x] | B | `activity.actors.user` | あなた | 自分 | G1 |
| [x] | B | `activity.triggers.plan_scheduled` | 定時のプラン作成 | プランの自動作成 | |
| [x] | B | `activity.triggers.action_approved` / `action_rejected` | 操作の承認 / 操作の却下 | 操作を許可 / 操作を不許可 | G3 |
| [x] | B | `activity.triggers.follow_up` | 自動の確認 | アシスタントからの確認 | G9. Matches `followUps.title`. |
| [x] | B | `activity.counts` | モデル N 回 · ツール N 回 · N トークン · ブロック N 回 | AI呼び出しN回・操作N回・Nトークン・ブロックN回 | G2, G4, G5 |
| [x] | C | `activity.planSaved` / `planSavedStep` / `toolSummaries.core_plan_*` | 第 N 版 | ver.N (or N回目) | 「第N版」 reads like a book edition. Optional. |
| [x] | B | `activity.errorCodes.invalid_input` | 入力が無効か、計画する項目がありません | 入力内容に誤りがあるか、プランを作成できる項目がありません | 「計画する項目」 is a literal translation. |
| [x] | B | `activity.errorCodes.internal` | 内部エラー | システムエラー | 「内部エラー」 is jargon; 「システムエラー」 is what JP users see elsewhere. |
| [ ] | B | `activity.errorCodes.not_configured` | モデルが未設定です | AIが設定されていません | G2 |
| [x] | B | `activity.errorCodes.rate_limited` | モデルが混み合っています | AIが混み合っています | G2 |
| [x] | B | `activity.showSteps` / `hideSteps` | ステップを表示 / 閉じる | 詳細を表示 / 詳細を隠す | The two labels should pair. ⚠︎ guide.pages.activity[2] |
| [x] | B | `activity.modelCall` | モデル呼び出し（${role}）… | AI呼び出し（${role}）… | G2 |
| [x] | B | `activity.toolOutcomes.invalid_args` | 拒否 | 無効な指定 | 「拒否」 sounds like a refusal by the user. |
| [x] | C | `activity.toolOutcomes.not_found` | 見つからない | 対象なし | A noun phrase fits a status label better. |
| [x] | B | `activity.arguments` | 引数 | 詳細データ | G2 |
| [x] | B | `activity.approvalRequested` | 確認をリクエスト | 確認を依頼 | |
| [x] | B | `activity.approvalResolved.*` | あなたが承認 / あなたが却下 / 承認したが実行に失敗 / 待機中 / 期限切れ | 許可済み / 不許可 / 許可後に実行失敗 / 確認待ち / 期限切れ | G1, G3. 「待機中」 should match the 「確認待ち」 status. |
| [x] | B | `activity.actionStatus.*` | 承認済み / 却下済み / 承認後に実行失敗 | 許可済み / 不許可 / 許可後に実行失敗 | G3 |
| [x] | B | `activity.guardrails.invalid_args` | ブロック：引数またはルールで許可されていません | ブロック：許可されていない内容です | G2 |
| [x] | B | `activity.guardrails.unknown_tool` | ブロック：存在しない、またはオフのツールです | ブロック：利用できない操作です（機能がオフなど） | G2 |
| [x] | B | `activity.guardrails.token_budget` | ブロック：1 日のトークン上限を超えています | ブロック：1日のAI利用上限を超えています | G2 |
| [x] | B | `activity.followUpRaised` | 確認を表示 | アシスタントから確認 | 「確認を表示」 sounds like a button. |

## 13. `planNotes`

| ✓ | Pri | Key | Current | Proposed | Reason |
|---|---|---|---|---|---|
| [x] | A | `planNotes.diet_main_meal_count` | 主食事（朝食・昼食・夕食）は 3 食必要ですが、${count} 食です | 朝食・昼食・夕食の3食が必要ですが、${count}食しかありません | 「主食事」 is not a Japanese word (it's a translation of "main meals"). |
| [x] | B | `planNotes.diet_kcal_off_target` | 1 日のエネルギー ${actual} kcal が… | 1日の合計カロリー ${actual} kcal が… | Consumer apps say カロリー. エネルギー is fine in the nutrition-label context (`macroViolations`). |
| [x] | B | `planNotes.diet_guest_scale_clamped` | ${guest}さんの盛り付け倍率を… | ${guest}さんの取り分け量の倍率を… | G9 |
| [x] | B | `planNotes.training_rest_day_too_hard` | 休養日は低強度のストレッチや可動域トレーニングのみにしてください | 休養日はストレッチなどの軽い運動だけにしてください | 「可動域トレーニング」 is specialist language. |
| [x] | B | `planNotes.multimember_out_of_age_range` | 「${name}」さんは… | ${name}さんは… | Putting 「」 around a name followed by さん looks odd. |
| [x] | C | `planNotes.stock_expiring_unused` | 在庫の「X」は D に期限切れになりますが、プランで使われていません | 在庫の「X」は D に期限が切れますが、プランで使われていません | 「期限切れになります」 is slightly awkward. |

## 14. `followUps`

| ✓ | Pri | Key | Current | Proposed | Reason |
|---|---|---|---|---|---|
| [x] | B | `followUps.hint` | プランと記録から自動で見つけました。タップで答えられます。 | プランと記録をもとにお知らせしています。タップして回答できます。 | 「見つけました」 has no object (what was found?). |
| [x] | B | `followUps.answers.core_dismiss` | 今はいい | あとで | 「今はいい」 is blunt for a button. |
| [ ] | C | `followUps.answers.diet_log_planned` | プラン通り食べた | プランどおり食べた | 「通り」 is usually written in hiragana in UI text. Optional. |

## 15. `toolSummaries` / `tools`

| ✓ | Pri | Key | Current | Proposed | Reason |
|---|---|---|---|---|---|
| [x] | B | `toolSummaries.core_preference_added` | …を記憶しました：… | …を登録しました：… | G9. 「記憶しました」 sounds odd for an app. |
| [x] | B | `toolSummaries.multimember_dislike_added` | …苦手な食べ物を記憶しました：… | …苦手な食べ物を登録しました：… | G9 |
| [x] | B | `toolSummaries.diet_label_calculated` | 表示値から ${g} g 分を換算しました（摂取記録なし） | 栄養成分表示から ${g}g 分を計算しました（食事記録には追加していません） | 「表示値」 is unclear; 「摂取記録なし」 is terse. |
| [x] | B | `toolSummaries.multimember_member_archived` | メンバーを外しました：… | メンバーを削除しました：… | G9. The button and the approval preview both say 削除. |
| [x] | B | `tools.core_add_preference` | 好みの記憶 | 好みの登録 | G9 |
| [x] | B | `tools.multimember_add_dislike` | メンバーの苦手な食べ物の記憶 | メンバーの苦手な食べ物の登録 | G9 |
| [x] | B | `tools.multimember_archive_member` | メンバーを外す | メンバーの削除 | G9. Also, every other `tools.*` entry is a noun phrase. |
| [x] | C | `tools.diet_calculate_label` | 栄養表示の換算 | 栄養成分表示の計算 | 「栄養成分表示」 is the official JP term. |

## 16. `settings`

| ✓ | Pri | Key | Current | Proposed | Reason |
|---|---|---|---|---|---|
| [x] | C | `settings.sections.preferences` | 好み | 好み・要望 | The section holds both kinds (苦手 / 要望). |
| [x] | B | `settings.features.followUps` | 自動の確認 | アシスタントからの確認 | G9 |
| [x] | B | `settings.features.followUpTimes` | チェック時刻 | 確認する時刻 | Also used in `errors.check_time_before_day_boundary`. |
| [ ] | C | `settings.features.names.multimember` | 一緒に食べる食事 | 家族・同居人との食事 | Makes the connection to the 「メンバー」 page clear. |
| [x] | B | `settings.preferences.hint` | 苦手な食べ物や、プランに反映してほしいことです。チャットで伝えた内容もここに保存されます。設定した期間だけ反映されます。 | 苦手な食べ物や、プランに反映してほしい要望を登録できます。チャットで伝えた内容もここに保存され、設定した期間だけプランに反映されます。 | The first sentence has no subject. |
| [x] | B | `settings.foodData.hint` | データソースは優先度の高い順に使われ、最初に一致したソースが採用されます。 | 食品データは上にあるものから優先して使われます。 | Plain language. |
| [x] | C | `settings.foodData.locked` | 順序固定、無効化や削除はできません | 並び順は固定です。無効化・削除はできません | Fragment → sentence. |
| [x] | C | `settings.foodData.colProblem` | 問題 | 内容 | Column header for errors/warnings. |
| [x] | C | `settings.foodData.sampleTitle` | 例 | プレビュー | It shows rows from the user's file, not an example. Optional. |

## 17. `members`

| ✓ | Pri | Key | Current | Proposed | Reason |
|---|---|---|---|---|---|
| [x] | C | `members.form.editTitle` | ${name} を編集 | ${name}さんの情報を編集 | Matches さん elsewhere. |
| [x] | B | `members.shared.with` | 一緒に：${names} · 計 ${n} 人前 | 一緒に食べる人：${names}（計${n}人分） | 「一緒に：」 is a fragment. G5 |
| [x] | C | `members.shared.guest` | ${name} 約 ${kcal} kcal | ${name}さん 約${kcal} kcal | さん |
| [x] | B | `members.shared.estimateNote` | それぞれ体重を保つのに必要な量で見積もった盛り付け量（目安） | 各自の体重維持に必要な量から計算した取り分け量の目安です | G9 |
| [x] | C | `members.intro`, `settings.features.descriptions.multimember` | …一人ずつ盛り付け量を決めます。 | …一人ずつ取り分け量を決めます。 | G9 |

## 18. `errors`

| ✓ | Pri | Key | Current | Proposed | Reason |
|---|---|---|---|---|---|
| [ ] | A | `errors.not_configured` | モデルが設定されていません。先に設定でモデルを設定してください。 | 現在AIを利用できません。管理者にお問い合わせください。 | There's no model setting in the web app's Settings, so the user can't follow this. (Also 「設定で…設定」 repeats itself.) |
| [ ] | A | `errors.llm_auth` | モデルの認証に失敗しました。API Key が正しいか確認してください。 | AIサービスに接続できませんでした。管理者にお問い合わせください。 | Users never see an API key. |
| [x] | B | `errors.internal` | 内部エラーが発生しました。もう一度お試しください。 | エラーが発生しました。しばらく時間をおいて、もう一度お試しください。 | 「内部エラー」 is jargon. |
| [x] | B | `errors.llm_rate_limited` | リクエストが多すぎます。しばらくしてから再試行してください。 | アクセスが集中しています。しばらく時間をおいて、もう一度お試しください。 | Standard JP phrasing. G8 |
| [x] | B | `errors.llm_timeout` | リクエストがタイムアウトしました。… | 応答に時間がかかりすぎたため中断しました。もう一度お試しください。 | 「タイムアウト」 is understood but technical. |
| [x] | B | `errors.llm_unavailable` | モデルのサービスで問題が発生しました。… | AIサービスで一時的な問題が発生しています。しばらく時間をおいて、もう一度お試しください。 | G2 |
| [x] | B | `errors.plan_blocked` | 生成したプランが安全性の確認を通過しなかったため、保存していません。既存のプランは変更されていません。条件を調整して再試行してください。 | 作成したプランが安全チェックを通らなかったため、保存しませんでした（今のプランはそのままです）。要望を変えて、もう一度お試しください。 | Shorter; 「条件を調整」 is vague. |
| [x] | A | `errors.age_out_of_range` | 年齢は 16〜100 歳の範囲である必要があります（生年月日から計算した年齢：…）。 | ご利用いただけるのは16〜100歳の方です（生まれた年月から計算した年齢：${got}歳）。 | 「〜である必要があります」 is translationese. Only year and month are collected, so 生年月日 is wrong. |
| [x] | A | `errors.member_age_out_of_range` | メンバーの年齢は16〜70歳です。この生年月日では${got}歳になります。 | 登録できるメンバーは16〜70歳です（生まれた年月から計算した年齢：${got}歳）。 | Same 生年月日 issue. |
| [x] | B | `errors.goal_rate_out_of_range` | 変化量は 0〜1.0 kg/週の範囲で… | 1週間あたりのペースは 0〜1.0 kg の範囲で… | Follows `onboarding.goal.ratePerWeek`. |
| [x] | B | `errors.plan_time_before_day_boundary` | プランの作成時刻（X）は Y より前にできません。もっと遅い時刻にしてください。 | プランの作成時刻は Y 以降に設定してください（入力：X）。 | The day boundary is now hidden in the UI, so phrase it as a plain rule. |
| [x] | B | `errors.check_time_before_day_boundary` | チェック時刻（X）は Y より前にできません。… | 確認する時刻は Y 以降に設定してください（入力：X）。 | Same reason. |
| [x] | B | `errors.instruction_empty` | 調整内容を入力してください。どのように調整するかを書いてください。 | どのように調整したいかを入力してください。 | The two sentences say the same thing. |
| [x] | C | `errors.preference_dates_reversed` | 開始日（A）は終了日（B）より後にできません。 | 開始日は終了日より前の日付にしてください（開始：A／終了：B）。 | State what to do, not what can't be done. |
| [x] | C | `errors.member_allergies_invalid` | アレルギーは20件まで、各1〜40文字です。 | アレルギーは20件まで、1件あたり40文字以内で入力してください。 | |
| [x] | B | `errors.too_many_attempts` | 試行回数が多すぎます。… | ログインの試行回数が上限に達しました。しばらく時間をおいて、もう一度お試しください。 | Standard phrasing. |
| [x] | B | `errors.too_many_requests` | リクエストが多すぎます。少し待ってから再試行してください。 | アクセスが集中しています。しばらく時間をおいて、もう一度お試しください。 | G8 |
| [x] | B | `errors.unknown_endpoint` | 該当する API が見つかりません。 | お探しの機能が見つかりません。ページを再読み込みしてください。 | 「API」 is jargon. |
| [x] | C | `errors.bad_request` | リクエストの内容に誤りがあります。 | 送信内容に誤りがあります。 | |

## 19. `importMessages` / `macroViolations`

These are for the CSV/JSON import, which is a power-user feature, so terms like 配列 and オブジェクト are acceptable. Only minor items:

| ✓ | Pri | Key | Current | Proposed | Reason |
|---|---|---|---|---|---|
| [x] | C | `importMessages.not_utf8` | …再試行してください | …もう一度お試しください | G8 |
| [x] | C | `importMessages.json_item_not_object` | ${line} 番目の項目が… | ${line}番目の項目が… | G4 (applies across this section) |

---

## Not changing (checked and OK)

- `mealType` 朝食・昼食・夕食・間食: same as あすけん.
- たんぱく質 (hiragana), 炭水化物, 脂質, 食物繊維: match あすけん, MyFitnessPal JP and the MHLW tables.
- 買い物リスト, 在庫, 期限切れ, 元に戻す: match the fridge-stock and shopping apps.
- `followUps.mealMissed`, `followUps.dayOffTarget.*`: natural.
- `settings.account.*`: standard account-deletion wording.
- `importMessages.trace_values` 「Tr/微量」: matches the notation in 日本食品標準成分表.
