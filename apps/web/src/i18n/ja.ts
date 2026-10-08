import type { Strings } from "./index";

/** Japanese UI strings (the default locale). Same keys as `./zh-CN.ts`. */
export const ja: Strings = {
  app: {
    name: "HumanHealthHandler",
  },
  common: {
    birthDate: { year: "年", month: "月", day: "日" },
    next: "次へ",
    back: "戻る",
    finish: "完了",
    finishing: "完了しています…",
    save: "保存",
    saving: "保存中…",
    cancel: "キャンセル",
    loading: "読み込み中…",
    optional: "任意",
    delete: "削除",
    confirmDelete: "削除する",
    retry: "再試行",
    networkError: "サーバーに接続できません。ネットワークを確認してから再試行してください。",
    logout: "ログアウト",
    listSeparator: "、",
    withOptional: (label: string) => `${label}（任意）`,
    labelled: (label: string, detail: string) => `${label}：${detail}`,
    parenthesized: (text: string) => `（${text}）`,
  },
  auth: {
    loginTitle: "ログイン",
    registerTitle: "新規登録",
    username: "ユーザー名",
    usernameHint: "半角英数字と _ - . で 3〜32 文字、大文字と小文字は区別しません",
    password: "パスワード",
    passwordHint: "8 文字以上",
    confirmPassword: "パスワード（確認）",
    passwordMismatch: "パスワードが一致しません。",
    login: "ログイン",
    register: "登録",
    submitting: "お待ちください…",
    toRegister: "アカウントをお持ちでない方は新規登録",
    toLogin: "アカウントをお持ちの方はログイン",
    sessionExpired: "ログインの有効期限が切れました。もう一度ログインしてください。",
  },
  nav: {
    today: "今日",
    chat: "チャット",
    activity: "アシスタント履歴",
    nutrition: "栄養",
    stock: "在庫",
    settings: "設定",
    training: "トレーニング",
    members: "メンバー",
  },
  guide: {
    open: "使い方ガイド",
    skip: "スキップ",
    done: "はじめる",
    stepCounter: (n: number, total: number) => `${n} / ${total}`,
    steps: [
      {
        title: "HumanHealthHandler へようこそ",
        body: "HHH は食事とトレーニングを記録し、あなたの目標に合わせて毎日翌日のプランを作ります。各画面を簡単にご紹介します。",
      },
      {
        title: "今日：今日をひと目で",
        body: "今日の食事プラン（記録済みの食事には ✓）、今日のトレーニングプラン、買い物リストをまとめて表示します。ここは確認用で、変更は各機能の画面かチャットで行います。",
      },
      {
        title: "チャット：ひとことで記録",
        body: "食べたものや運動したことを話しかけるように伝えてください。例：「お昼にラーメンとビールを1杯」。カロリーと栄養素はプログラムが計算します。\n間違えたときもチャットで修正・削除できます。",
      },
      {
        title: "栄養：食事の記録と明日のプラン",
        body: "今日のカロリーとたんぱく質・炭水化物・脂質を確認し、記録の削除や体重の記録ができます。\n明日のプランは設定した時刻に自動で作られます。今すぐ作る、要望を書いて作り直す、プランを採用することもできます。",
      },
      {
        title: "トレーニングと在庫",
        body: "トレーニング画面では直近 7 日間の記録とトレーニングプランを、在庫画面では家にある食材を管理します。買い物リストの各項目は完了・キャンセルにできます。どちらも設定でオン・オフできます。",
      },
      {
        title: "設定",
        body: "プロフィールと目標、機能のオン・オフ、プランを作る時刻、苦手な食べ物などの好み、表示言語を変更できます。アシスタントは選んだ言語で返事をします。\nこのガイドは ? ボタンからいつでも見られます。",
      },
    ],
  },
  mealType: {
    breakfast: "朝食",
    lunch: "昼食",
    dinner: "夕食",
    snack: "間食",
  },
  onboarding: {
    stepLabel: (step: number, total: number) => `ステップ ${step} / ${total}`,
    steps: ["基本情報", "目標"],
    profile: {
      title: "基本情報",
      sex: "性別",
      sexOptions: { male: "男性", female: "女性" },
      birthDate: "生年月日",
      heightCm: "身長（cm）",
      weightKg: "現在の体重（kg）",
      activityLevel: "活動レベル",
      activityLevelHints: {
        sedentary: "座りがち — ほとんど運動せず、主にデスクワーク",
        light: "軽い活動 — 週 1〜3 日の軽い運動",
        moderate: "中程度の活動 — 週 3〜5 日の中強度の運動",
        active: "活発 — 週 6〜7 日の運動",
        very_active: "非常に活発 — 高強度のトレーニングや肉体労働",
      },
      dietaryPattern: "食事スタイル",
      dietaryPatternOptions: {
        omnivore: "制限なし",
        vegetarian: "ベジタリアン",
        vegan: "ヴィーガン",
        pescatarian: "ペスカタリアン",
      },
      allergies: "アレルギー食品",
      allergiesPlaceholder: "読点で区切って入力（例：えび、ピーナッツ）",
      dislikes: "苦手な食べ物",
      dislikesPlaceholder: "読点で区切って入力（例：パクチー）",
      mealsPerDay: "1 日の食事回数",
      medicalNotes: "健康状態のメモ",
      medicalNotesPlaceholder: "関係する健康状態があればご記入ください（任意）",
    },
    goal: {
      title: "目標",
      direction: "目標の方向",
      directionOptions: {
        lose: "減量",
        maintain: "維持",
        gain: "増量",
      },
      targetWeightKg: "目標体重（kg）",
      targetWeightPlaceholder: "空欄の場合は自動で設定",
      ratePerWeek: "1 週間あたりの変化量",
      rateOptions: {
        "0.25": "0.25 kg/週",
        "0.5": "0.5 kg/週",
        "0.75": "0.75 kg/週",
      },
      previewTitle: "1 日の目標カロリー（見込み）",
      previewKcal: (kcal: number) => `${kcal} kcal/日`,
      previewMacros: (protein: number, carbs: number, fat: number) =>
        `たんぱく質 ${protein}g · 炭水化物 ${carbs}g · 脂質 ${fat}g`,
      previewFloorApplied: "安全な下限値に合わせて調整しました。",
      previewError: "現在プレビューを計算できません。上の入力内容を確認してください。",
    },
    issues: {
      title: "次に進むには：",
      sexMissing: "性別を選択してください",
      birthDateMissing: "生年月日（年・月・日）を選択してください",
      heightMissing: "身長を入力してください",
      heightRange: (min: number, max: number) => `身長は ${min}〜${max} cm で入力してください`,
      weightMissing: "現在の体重を入力してください",
      weightRange: (min: number, max: number) => `体重は ${min}〜${max} kg で入力してください`,
    },
    errors: {
      required: "入力してください",
      saveFailed: "保存に失敗しました。もう一度お試しください。",
    },
  },
  today: {
    latestWeight: (kg: number) => `最新の体重 ${kg} kg`,
    noWeightYet: "体重の記録がまだありません",
    logWeight: "体重を記録",
    logWeightTitle: "体重を記録",
    logWeightPlaceholder: "kg",
    logWeightInvalid: "0 より大きい体重（kg）を入力してください",
    diet: {
      title: "今日の食事",
      kcalEatenOfTarget: (eaten: number, target: number) => `${eaten} / ${target} kcal`,
      kcalEaten: (eaten: number) => `${eaten} kcal`,
      ofTargetKcal: (target: number) => `/ ${target} kcal`,
      noTarget: "目標が未設定です。プロフィールと体重を登録してください",
      protein: "たんぱく質",
      carbs: "炭水化物",
      fat: "脂質",
      gramsShort: (g: number) => `${g}g`,
      servingsShort: (n: number) => `×${n}`,
      noMealsYet: "今日の食事はまだ記録されていません",
      estimatedBadge: "推定",
    },
    todayPlan: {
      title: "今日のプラン",
      eaten: "食べた",
      notYetEaten: "まだ",
    },
    tomorrowPlan: {
      title: "明日のプラン",
      noPlanYet: (time: string) => `${time} に自動で作成されます`,
      generateNow: "今すぐ作成",
      generating: "作成中…",
      accept: "プランを採用",
      regenerate: "作り直す",
      adjust: "調整",
      adjusting: "調整中…",
      adjustPrefill: "明日のプランを調整してください：",
      warningsTitle: "注意",
      medicalDisclaimer:
        "プロフィールに健康状態が記録されています。このプランは生活習慣の提案にすぎません。医師や管理栄養士の指示を優先してください。",
      dayTotal: (kcal: number, target: number) => `1 日の合計 ${kcal} / ${target} kcal`,
      trainingLine: (title: string, minutes: number) => `トレーニング：${title} · ${minutes} 分`,
      trainingRest: "トレーニング：休養日",
      trainingKcalAdded: (kcal: number) => `トレーニング分として ${kcal} kcal を追加`,
      generateFailed: "プランの作成に失敗しました。もう一度お試しください。",
      requestLabel: "今回のプランへの要望",
      requestPlaceholder: "今回のプランへの要望（任意）例：明日は残業なのでコンビニで買えるものに",
      requestPlaceholderWithPlan:
        "要望（任意）例：明日は残業なのでコンビニで買えるものに。「作り直す」は新しいプランを作り、「調整」は今のプランを直します。",
    },
    deleteMealConfirm: "この記録を削除しますか？",
  },
  overview: {
    dietTitle: "今日の食事プラン",
    noDietPlan: "今日の食事プランはありません。「栄養」画面で明日のプランを作れます。",
    noTrainingPlan: "今日のトレーニングプランはありません。",
  },
  stock: {
    hint: "チャットで「卵1パックと牛乳1Lを買った」のように伝えても追加できます。",
    autoConsume: "食事を記録したら在庫から自動で減らす",
    planFromStock: "在庫をもとにプランと買い物リストを作る",
    itemsTitle: "いまの在庫",
    empty: "在庫はありません",
    units: { g: "g", ml: "ml", piece: "個" },
    amount: (n: number, unit: string) => `${n} ${unit}`,
    bestBefore: (d: string) => `${d} まで`,
    expired: "期限切れ",
    quantityLabel: (name: string) => `${name} の残り`,
    add: "追加",
    adding: "追加中…",
    name: "名前",
    namePlaceholder: "例：卵",
    quantity: "数量",
    unit: "単位",
    expiresOn: "賞味期限",
    shoppingTitle: "買い物リスト",
    shoppingFor: (d: string) => `${d} のプランより`,
    noShopping: "まだプランがありません。明日のプランを作ると、買う食材がここに並びます。",
    nothingToBuy: "在庫で足りるので、買う物はありません。",
    inStock: (amount: string) => `在庫あり ${amount}`,
    complete: "完了",
    completeHint: "買えた：プランの量を在庫に追加します",
    cancel: "キャンセル",
    reopen: "元に戻す",
    statusDone: "購入済み",
    statusCancelled: "キャンセル済み",
    recentTitle: "最近使った分",
    recentEmpty: "食事を記録すると、使った在庫がここに表示されます。",
    usedToast: "記録した食事の分を在庫から差し引きました：",
    undo: "元に戻す",
    undoing: "元に戻しています…",
    undoHint: "在庫に戻します（食事の記録はそのまま残ります）",
    dismiss: "閉じる",
  },
  training: {
    title: "トレーニング",
    week: {
      title: "直近 7 日間",
      summary: (sessions: number, minutes: number) => `${sessions} 回 · ${minutes} 分`,
      kcal: (kcal: number) => `運動消費 約 ${kcal} kcal`,
      kcalHint: "基礎代謝を除いた値で、種目・時間・体重からアプリが推定しています",
      empty: "直近 7 日間のトレーニング記録はまだありません。チャットで「今日は 5 km を 30 分で走った」のように教えてください。",
      today: "今日",
      rest: "—",
      chatHint: "チャットで記録できます（例：「スクワット 60 kg で 8 回を 4 セットやった」）。",
    },
    todayPlan: {
      title: "今日のトレーニングプラン",
    },
    tomorrow: {
      title: (date: string) => `明日のトレーニングプラン（${date}）`,
      noPlan: "明日のプランはまだありません。",
      noTrainingSection: "明日のプランはトレーニング機能をオンにする前に作成されました。作り直すとトレーニングが加わります。",
      preferenceLabel: "明日はどんなトレーニングにしたいですか？（任意）",
      preferencePlaceholder: "例：脚の日、45 分／朝にランニング／休養日にしたい",
      savePreference: "保存",
      preferenceSaved: "保存しました",
      generate: "明日のプランを作成",
      regenerate: "この希望で作り直す",
      generating: "作成中…",
      dietNote: "明日の食事プランはこのトレーニングプランに合わせて組まれます。",
    },
    plan: {
      restDay: "休養日",
      untitled: "トレーニング",
      minutes: (minutes: number) => `${minutes} 分`,
      kcal: (kcal: number) => `約 ${kcal} kcal`,
      timeOfDay: {
        morning: "朝",
        midday: "昼",
        afternoon: "午後",
        evening: "夜",
      },
      intensity: {
        low: "低強度",
        moderate: "中強度",
        high: "高強度",
      },
    },
    exercise: {
      setsReps: (sets: number, reps: number) => `${sets}×${reps}`,
      sets: (sets: number) => `${sets} セット`,
      reps: (reps: number) => `${reps} 回`,
      weight: (kg: number) => `${kg} kg`,
      duration: (minutes: number) => `${minutes} 分`,
      distance: (km: number) => `${km} km`,
    },
    preferences: {
      title: "トレーニングの好み",
      goal: "目標",
      goalOptions: {
        general_fitness: "総合的な体力",
        strength: "筋力",
        muscle: "筋肥大",
        endurance: "持久力",
        fat_loss: "減量",
      },
      daysPerWeek: "週のトレーニング日数",
      sessionMinutes: "1 回の時間（分）",
      equipment: "器具・場所",
      equipmentPlaceholder: "例：ジム／自宅、ダンベルあり",
      notes: "メモ",
      notesPlaceholder: "例：左ひざに古傷があるのでジャンプは避けたい",
      invalid: "週の日数は 1〜7、1 回の時間は 10〜180 分の範囲で入力してください。",
    },
  },
  chat: {
    inputPlaceholder: "メッセージを入力（Enter で送信、Shift+Enter で改行）",
    send: "送信",
    sending: "送信中…",
    empty: "まだ会話がありません。何か話しかけてみましょう",
    thinking: "考え中",
    deepThinking: "じっくり考え中",
    sendFailed: "送信に失敗しました",
    notices: {
      empty_reply: "モデルから返答がありませんでした。もう一度お試しください。",
      too_many_steps: "処理の手順が多すぎたため停止しました。完了した操作は下のとおりです。残りはもう一度送ってください。",
    },
  },
  approvals: {
    title: "確認が必要です",
    hint: "アシスタントが次の変更を求めています。承認するまで実行されません。",
    approve: "承認",
    reject: "却下",
    approved: "承認して実行しました",
    rejected: "却下しました（変更なし）",
    expired: "期限切れです（変更なし）",
    failed: (reason: string) => `承認しましたが、実行に失敗しました：${reason}`,
    failedToResolve: "操作に失敗しました",
    previews: {
      update_allergies: (d: { add: string[]; remove: string[] }) =>
        [
          d.add.length > 0 ? `アレルギーに追加：${d.add.join("、")}` : "",
          d.remove.length > 0 ? `アレルギーから削除：${d.remove.join("、")}` : "",
        ]
          .filter((s) => s !== "")
          .join("／"),
      add_preference: (d: {
        preference_kind: "dislike" | "general";
        text: string;
        valid_from: string | null;
        valid_until: string | null;
        permanent: boolean;
      }) =>
        `${d.preference_kind === "dislike" ? "苦手な食べ物" : "好み"}を保存：${d.text}` +
        (d.permanent
          ? "（ずっと）"
          : d.valid_from || d.valid_until
            ? `（${d.valid_from ?? ""}〜${d.valid_until ?? ""}）`
            : "（1か月）"),
      remove_preference: (d: { preference_kind: "dislike" | "general"; text: string }) =>
        `${d.preference_kind === "dislike" ? "苦手な食べ物" : "好み"}を削除：${d.text}`,
      delete_past_entry: (d: { logical_date: string; description: string | null }) =>
        `${d.logical_date} の記録を削除${d.description ? `：${d.description}` : ""}`,
      archive_member: (d: { name: string }) => `メンバーを削除：${d.name}`,
      update_member_allergies: (d: { name: string; allergies: string[] }) =>
        `${d.name} さんのアレルギーを変更：${d.allergies.length > 0 ? d.allergies.join("、") : "なし"}`,
      add_member: (d: { name: string; allergies: string[] }) =>
        `メンバーを追加：${d.name}（アレルギー：${d.allergies.length > 0 ? d.allergies.join("、") : "なし"}）`,
      update_training_notes: (d: { notes: string; previous_notes: string; other_changes: boolean }) =>
        `トレーニングのメモ（けが等）を変更：${d.notes || "なし"}` +
        (d.previous_notes ? `（変更前：${d.previous_notes}）` : "") +
        (d.other_changes ? "、ほかのトレーニング設定も変更" : ""),
      update_nutrition: (d) =>
        `${d.brand ? `${d.brand} ` : ""}${d.name} の栄養値を変更（${d.basis === "per_100g" ? "100g あたり" : "1 食あたり"}）：` +
        `${d.before ? `${d.before.kcal} → ` : ""}${d.after.kcal} kcal · ` +
        `たんぱく質 ${d.after.protein_g}g · 炭水化物 ${d.after.carbs_g}g · 脂質 ${d.after.fat_g}g` +
        d.reasons
          .map((r) => (r === "large_change" ? "（大きな変更）" : "（カロリーが三大栄養素と合いません）"))
          .join(""),
      add_food_alias: (d: { alias: string; food: string }) => `「${d.alias}」を「${d.food}」として記録`,
      remove_food_alias: (d: { alias: string; target: string }) =>
        `「${d.alias}」を「${d.target}」として扱うのをやめる`,
    },
    unknown: (tool: string) => `操作を実行：${tool}`,
  },
  activity: {
    hint: "アシスタントがあなたの代わりに行ったことをすべて記録しています。誰が始めたか、何を呼び出したか、何が止められ、何があなたの確認待ちかが分かります。",
    period: (days: number) => `直近 ${days} 日`,
    stats: {
      byUser: "あなたが開始",
      byScheduler: "自動実行",
      approvals: "確認リクエスト",
      approvalsDetail: (approved: number, rejected: number) => `承認 ${approved} · 却下 ${rejected}`,
      guardrails: "安全ブロック",
      tokens: "モデルのトークン",
      last24h: (used: number, budget: number | null) =>
        budget === null ? `直近 24 時間 ${used}` : `直近 24 時間 ${used} / 上限 ${budget}`,
    },
    filterAll: "すべて",
    actors: { user: "あなた", scheduler: "自動" },
    triggers: {
      chat_turn: "チャット",
      plan_scheduled: "定時のプラン作成",
      plan_manual: "プラン作成",
      plan_revision: "プランの調整",
      action_approved: "操作の承認",
      action_rejected: "操作の却下",
    },
    statuses: {
      running: "実行中",
      succeeded: "完了",
      needs_approval: "確認待ち",
      failed: "失敗",
      interrupted: "中断",
    },
    counts: (o: { model_calls: number; tool_calls: number; tokens: number; guardrails: number }) =>
      [
        `モデル ${o.model_calls} 回`,
        `ツール ${o.tool_calls} 回`,
        `${o.tokens} トークン`,
        o.guardrails > 0 ? `ブロック ${o.guardrails} 回` : "",
      ]
        .filter((s) => s !== "")
        .join(" · "),
    planSaved: (date: string, version: number) => `${date} のプラン 第 ${version} 版`,
    errorCodes: {
      invalid_input: "入力が無効か、計画する項目がありません",
      internal: "内部エラー",
      budget_exceeded: "1 日のトークン上限に達しました",
      busy: "別のプランを作成中です",
      not_configured: "モデルが未設定です",
      action_not_pending: "この操作は処理済みか期限切れです",
      rate_limited: "モデルが混み合っています",
    } as Record<string, string>,
    errorCode: (code: string) => `エラー：${code}`,
    empty: "まだ記録がありません。",
    loadMore: "さらに読み込む",
    showSteps: "ステップを表示",
    hideSteps: "閉じる",
    roles: { conversation: "会話", parser: "解析", planner: "プラン" },
    modelCall: (role: string, model: string, tokens: number, seconds: string, ok: boolean) =>
      `モデル呼び出し（${role}）${model} · ${tokens} トークン · ${seconds} 秒${ok ? "" : " · 失敗"}`,
    toolOutcomes: {
      ran: "実行",
      invalid_args: "拒否",
      not_found: "見つからない",
      failed: "エラー",
    },
    arguments: "引数",
    approvalRequested: "確認をリクエスト",
    approvalResolved: {
      approved: "あなたが承認",
      rejected: "あなたが却下",
      failed: "承認したが実行に失敗",
      pending: "待機中",
      expired: "期限切れ",
    },
    fromEarlierRun: "（以前の会話から）",
    actionStatus: {
      pending: "まだ確認待ちです",
      approved: "承認済み",
      rejected: "却下済み",
      failed: "承認後に実行失敗",
      expired: "期限切れ（未実行）",
    },
    guardrails: {
      invalid_args: "ブロック：引数またはルールで許可されていません",
      unknown_tool: "ブロック：存在しない、またはオフのツールです",
      token_budget: "ブロック：1 日のトークン上限を超えています",
      iteration_cap: "停止：ステップが多すぎます",
    },
    planValidation: (d: { violations: readonly unknown[]; retried: boolean; violations_left: number }) =>
      d.violations.length === 0
        ? "プランのチェック：問題なし"
        : d.violations_left === 0
          ? `プランのチェック：${d.violations.length} 件の問題を検出し、作り直してすべて解消`
          : `プランのチェック：${d.violations.length} 件の問題を検出、${d.violations_left} 件は未解消（注意として表示）`,
    planSavedStep: (date: string, version: number) => `${date} のプランを保存しました（第 ${version} 版）`,
  },
  planNotes: {
    unmet: (text: string) => `未解消：${text}`,
    core_section_missing: (d: { section: string }) => `プランに「${d.section}」の項目がありません`,
    diet_invalid_structure: "食事プランの形式が正しくありません",
    diet_item_unnamed: "名前のない食品があります",
    diet_grams_out_of_range: (d: { name: string; grams: number; min: number; max: number }) =>
      `「${d.name}」の量 ${d.grams} g が ${d.min}〜${d.max} g の範囲外です`,
    diet_allergen: (d: {
      meal_type: "breakfast" | "lunch" | "dinner" | "snack";
      meal_title: string;
      name: string;
      allergen: string;
    }) =>
      `${{ breakfast: "朝食", lunch: "昼食", dinner: "夕食", snack: "間食" }[d.meal_type]}「${d.meal_title}」の「${d.name}」にアレルゲン「${d.allergen}」が含まれています`,
    diet_disliked: (d: { name: string; term: string }) =>
      `「${d.name}」に苦手な「${d.term}」が含まれています`,
    diet_main_meal_count: (d: { count: number }) =>
      `主食事（朝食・昼食・夕食）は 3 食必要ですが、${d.count} 食です`,
    diet_meal_count: (d: { count: number; max: number }) =>
      `食事の回数は 3〜${d.max} 回にしてください（現在 ${d.count} 回）`,
    diet_kcal_off_target: (d: { actual: number; target: number; low: number; high: number }) =>
      `1 日のエネルギー ${d.actual} kcal が目標 ${d.target} kcal の ±8%（${d.low}〜${d.high}）を外れています`,
    diet_protein_low: (d: { actual: number; target: number }) =>
      `たんぱく質 ${d.actual} g が目標 ${d.target} g の 90% を下回っています`,
    diet_fat_off_target: (d: { actual: number; target: number }) =>
      `脂質 ${d.actual} g が目標 ${d.target} g から 20% 以上ずれています`,
    diet_carbs_off_target: (d: { actual: number; target: number }) =>
      `炭水化物 ${d.actual} g が目標 ${d.target} g から 20% 以上ずれています`,
    diet_fiber_low: (d: { actual: number; target: number }) =>
      `食物繊維 ${d.actual} g が目標 ${d.target} g の 70% を下回っています`,
    diet_guest_allergen: (d: {
      meal_type: "breakfast" | "lunch" | "dinner" | "snack";
      meal_title: string;
      name: string;
      guest: string;
      allergen: string;
    }) =>
      `${{ breakfast: "朝食", lunch: "昼食", dinner: "夕食", snack: "間食" }[d.meal_type]}「${d.meal_title}」の「${d.name}」に${d.guest}さんのアレルゲン「${d.allergen}」が含まれています`,
    diet_guest_disliked: (d: { name: string; guest: string; term: string }) =>
      `「${d.name}」に${d.guest}さんの苦手な「${d.term}」が含まれています`,
    diet_shared_pattern: (d: {
      meal_type: "breakfast" | "lunch" | "dinner" | "snack";
      meal_title: string;
      pattern: "omnivore" | "vegetarian" | "vegan" | "pescatarian";
      name: string;
      term: string;
    }) =>
      `${{ breakfast: "朝食", lunch: "昼食", dinner: "夕食", snack: "間食" }[d.meal_type]}「${d.meal_title}」は一緒に食べる食事のため${{ omnivore: "制限なし", vegetarian: "ベジタリアン", vegan: "ヴィーガン", pescatarian: "ペスカタリアン" }[d.pattern]}に合わせる必要があります：「${d.name}」に「${d.term}」が含まれています`,
    diet_guest_scale_clamped: (d: { guest: string; scale: number; min: number; max: number }) =>
      `${d.guest}さんの盛り付け倍率を ${d.min}〜${d.max} の範囲に制限しました（${d.scale}）`,
    stock_invalid_structure: "在庫プランの形式が正しくありません",
    stock_not_in_stock: (d: { name: string }) => `「${d.name}」は在庫にないため使えません`,
    stock_wrong_unit: (d: { name: string; unit: "g" | "ml" | "piece"; planned_unit: "g" | "ml" | "piece" }) =>
      `「${d.name}」の単位は${{ g: "g", ml: "ml", piece: "個" }[d.unit]}で、${{ g: "g", ml: "ml", piece: "個" }[d.planned_unit]}ではありません`,
    stock_over_quantity: (d: { name: string; amount: number; quantity: number; unit: "g" | "ml" | "piece" }) =>
      `「${d.name}」の使用量 ${d.amount} ${{ g: "g", ml: "ml", piece: "個" }[d.unit]}が在庫 ${d.quantity} ${{ g: "g", ml: "ml", piece: "個" }[d.unit]}を超えています`,
    stock_expiring_unused: (d: { name: string; expires_on: string }) =>
      `在庫の「${d.name}」は ${d.expires_on} に期限切れになりますが、プランで使われていません`,
    training_invalid_structure: "トレーニングプランの形式が正しくありません",
    training_invalid_exercise: (d: {
      name: string;
      problem: "name_empty" | "name_too_long" | "sets" | "reps" | "weight" | "duration" | "distance";
    }) =>
      `「${d.name}」：${
        {
          name_empty: "種目名を入力してください",
          name_too_long: "種目名が長すぎます",
          sets: "セット数は 1〜20 にしてください",
          reps: "回数は 1〜200 にしてください",
          weight: "重量は 0〜500 kg にしてください",
          duration: "時間は 0〜300 分にしてください",
          distance: "距離は 0〜300 km にしてください",
        }[d.problem]
      }`,
    training_rest_day_too_hard: "休養日は低強度のストレッチや可動域トレーニングのみにしてください",
    training_no_exercises: "トレーニング日には種目を 1 つ以上入れてください",
    training_too_long: (d: { minutes: number; max: number }) =>
      `トレーニング時間 ${d.minutes} 分が上限の ${d.max} 分を超えています`,
    training_over_usual: (d: { minutes: number; usual: number }) =>
      `トレーニング時間 ${d.minutes} 分がいつもの ${d.usual} 分を大きく超えています`,
    training_hard_after_break: "直近 7 日間にトレーニング記録がありません。低〜中強度から始めましょう",
    training_big_jump: (d: { longest: number }) =>
      `直近 7 日間で最長のトレーニング（${d.longest} 分）より 1.5 倍以上長くなっています。少しずつ増やしましょう`,
    training_third_hard_day: "前の 2 日間が高強度でした。強度を下げるか休養しましょう",
    multimember_out_of_age_range: (d: { name: string; age: number; min: number; max: number }) =>
      `「${d.name}」さんは ${d.min}〜${d.max} 歳の範囲外（${d.age} 歳）のため、一緒に食べる食事に含めていません`,
  },
  toolSummaries: {
    core_plan_viewed: (d: { date: string }) => `${d.date} のプランを確認しました`,
    core_plan_generated: (d: { date: string; version: number }) =>
      `${d.date} のプランを作成しました（第 ${d.version} 版）`,
    core_plan_revised: (d: { date: string; version: number }) =>
      `${d.date} のプランを調整しました（第 ${d.version} 版）`,
    core_weight_logged: (d: { kg: number }) => `体重 ${d.kg} kg を記録しました`,
    core_allergies_updated: (d: { allergies: string[] }) =>
      `アレルギー食品を更新しました：${d.allergies.length > 0 ? d.allergies.join("、") : "なし"}`,
    core_preference_added: (d: {
      preference_kind: "dislike" | "general";
      text: string;
      valid_from: string | null;
      valid_until: string | null;
    }) =>
      `${d.preference_kind === "dislike" ? "苦手な食べ物" : "好み"}を記憶しました：${d.text}` +
      (d.valid_until ? `（${d.valid_until} まで）` : ""),
    core_preference_removed: "好みを削除しました",
    core_entry_deleted: "記録を削除しました",
    diet_meal_logged: (d: { meal_type: "breakfast" | "lunch" | "dinner" | "snack"; kcal: number }) =>
      `${{ breakfast: "朝食", lunch: "昼食", dinner: "夕食", snack: "間食" }[d.meal_type]}を記録しました · ${d.kcal} kcal`,
    diet_intake_viewed: (d: { date: string }) => `${d.date} の食事記録を確認しました`,
    diet_food_updated: (d: { name: string }) => `食品を更新しました：${d.name}`,
    diet_dish_updated: (d: { name: string }) => `料理を更新しました：${d.name}`,
    diet_alias_added: (d: { alias: string; food: string }) =>
      `別名を追加しました：${d.alias} → ${d.food}`,
    diet_alias_removed: (d: { alias: string }) => `別名を削除しました：${d.alias}`,
    stock_added: (d: { names: string[] }) => `在庫に追加しました：${d.names.join("、")}`,
    stock_viewed: "在庫を確認しました",
    stock_updated: (d: { name: string }) => `在庫を更新しました：${d.name}`,
    training_session_logged: (d: { title: string; minutes: number }) =>
      `トレーニングを記録しました：${d.title} ${d.minutes} 分`,
    training_log_viewed: (d: { start_date: string; end_date: string }) =>
      `${d.start_date}〜${d.end_date} のトレーニング記録を確認しました`,
    training_day_preference_set: (d: { date: string; cleared: boolean }) =>
      d.cleared
        ? `${d.date} のトレーニングの希望を消去しました`
        : `${d.date} のトレーニングの希望を保存しました`,
    training_preferences_updated: "トレーニング設定を更新しました",
    multimember_members_viewed: (d: { count: number }) => `メンバーを確認しました（${d.count} 人）`,
    multimember_member_added: (d: { name: string }) => `メンバーを追加しました：${d.name}`,
    multimember_member_updated: (d: { name: string }) => `メンバーを更新しました：${d.name}`,
    multimember_member_archived: (d: { name: string }) => `メンバーを外しました：${d.name}`,
    multimember_attendance_set: (d: {
      name: string;
      meal_type: "breakfast" | "lunch" | "dinner";
      present: boolean;
      date: string | null;
    }) =>
      `${d.name}・${d.date ? `${d.date} の` : "いつもの"}${{ breakfast: "朝食", lunch: "昼食", dinner: "夕食" }[d.meal_type]}：${d.present ? "一緒に食べる" : "一緒に食べない"}`,
    multimember_attendance_cleared: (d: {
      name: string;
      meal_type: "breakfast" | "lunch" | "dinner";
      date: string;
    }) =>
      `${d.name}・${d.date} の${{ breakfast: "朝食", lunch: "昼食", dinner: "夕食" }[d.meal_type]}：いつもの予定に戻しました`,
    multimember_dislike_added: (d: { name: string; text: string }) =>
      `${d.name} さんの苦手な食べ物を記憶しました：${d.text}`,
    multimember_dislike_removed: "苦手な食べ物を 1 件削除しました",
  },
  tools: {
    core_get_plan: "プランの確認",
    core_generate_plan: "プランの作成",
    core_revise_plan: "プランの調整",
    core_log_weight: "体重の記録",
    core_update_allergies: "アレルギー食品の更新",
    core_add_preference: "好みの記憶",
    core_remove_preference: "好みの削除",
    core_delete_entry: "記録の削除",
    diet_log_meal: "食事の記録",
    diet_get_intake: "食事記録の確認",
    diet_update_food: "食品の更新",
    diet_update_dish: "料理の更新",
    diet_add_alias: "別名の追加",
    diet_remove_alias: "別名の削除",
    stock_add: "在庫の追加",
    stock_list: "在庫の確認",
    stock_update: "在庫の更新",
    training_log_session: "トレーニングの記録",
    training_get_log: "トレーニング記録の確認",
    training_set_day_preference: "日ごとのトレーニングの希望",
    training_update_preferences: "トレーニング設定の更新",
    multimember_list_members: "メンバーの確認",
    multimember_add_member: "メンバーの追加",
    multimember_update_member: "メンバーの更新",
    multimember_archive_member: "メンバーを外す",
    multimember_set_attendance: "食事の参加予定の設定",
    multimember_clear_attendance_override: "参加予定をいつも通りに戻す",
    multimember_add_dislike: "メンバーの苦手な食べ物の記憶",
    multimember_remove_dislike: "メンバーの苦手な食べ物の削除",
  } as Record<string, string>,
  settings: {
    saveSucceeded: "保存しました",
    saveFailed: "保存に失敗しました。もう一度お試しください。",
    sections: {
      profileGoal: "プロフィールと目標",
      foodData: "食品データ",
      plan: "プラン",
      preferences: "好み",
      account: "アカウント",
      language: "言語",
      features: "機能",
    },
    features: {
      hint: "オプション機能をオンにすると、サイドバーに表示され、毎日のプランに加わります。",
      activityTab: "「アシスタント履歴」ページを表示",
      activityTabHint: "アシスタントの操作は常に記録されます。ここではページをサイドバーに表示するかだけを選びます。",
      names: {
        training: "トレーニング記録とプラン",
        stock: "在庫管理",
        multimember: "一緒に食べる食事",
      } as Record<string, string>,
      descriptions: {
        training:
          "チャットでトレーニングを記録し、好みと直近 7 日間の記録から毎日のトレーニングプランを作成します。翌日の食事プランはトレーニングに合わせて組まれます。",
        stock:
          "家にある食材を記録します。記録した食事は在庫から自動で差し引かれ、プランは在庫を優先して使い、買い物リストも作ります。",
        multimember:
          "一緒に食べる人（16〜70歳）を登録します。一緒に食べる食事は全員分を同じ献立で作り、それぞれのアレルギーと苦手な食べ物を避け、体重を保つのに必要な量で一人ずつ盛り付け量を決めます。",
      } as Record<string, string>,
    },
    language: {
      hint: "切り替えるとページが再読み込みされます。",
    },
    account: {
      loggedInAs: (username: string) => `ログイン中：${username}`,
      deleteTitle: "アカウントを削除",
      deleteHint:
        "このアカウントとすべてのデータ（食事記録、プラン、会話、好み）が完全に削除され、元に戻せません。",
      passwordToConfirm: "確認のためパスワードを入力",
      deleteButton: "アカウントを完全に削除",
      deleting: "削除中…",
    },
    plan: {
      planTime: "プランの作成時刻",
      dayBoundary: "1 日の区切りの時刻",
    },
    preferences: {
      hint: "苦手な食べ物や、プランに反映してほしいことです。チャットで伝えた内容もここに保存されます。設定した期間だけ反映されます。",
      empty: "まだ登録されていません",
      kinds: { dislike: "苦手", general: "要望" },
      kind: "種類",
      text: "内容",
      dislikePlaceholder: "例：パクチー",
      generalPlaceholder: "例：平日の昼は社員食堂",
      validFrom: "開始日",
      validUntil: "終了日",
      permanent: "期限なし",
      validityHint: "終了日が空欄の場合、開始日（空欄なら今日）から 1 か月間有効です。",
      add: "追加",
      adding: "追加中…",
      from: (d: string) => `${d} から`,
      until: (d: string) => `${d} まで`,
      between: (from: string, until: string) => `${from} 〜 ${until}`,
      expired: "期限切れ",
    },
    foodData: {
      hint: "データソースは優先度の高い順に使われ、最初に一致したソースが採用されます。",
      importButton: "データをインポート…",
      formatHelp:
        "CSV または JSON 形式の食品成分表をインポートできます。ファイルに別名の列を含めることもできます。エネルギーの列は既定で kcal として読み込みます。kJ の場合は見出しを kj または energy_kj にしてください。チェーン店のメニューなど 1 食単位の料理もインポートできます：basis 列で「100g」の食品行と「serving」の料理行を区別し、料理行では brand 列でチェーン名を指定できます。料理行の basis_g 列は 1 食分の重さ（g）を表します。",
      empty:
        "まだデータセットがインポートされていません。CSV または JSON 形式の食品成分表をインポートできます。ファイルに別名の列を含めることもできます。エネルギーの列は既定で kcal として読み込みます。kJ の場合は見出しを kj または energy_kj にしてください。",
      kindManual: "手動入力",
      kindEstimate: "AI 推定",
      kindDataset: (date: string) => `データセット · ${date} にインポート`,
      kindDatasetPlain: "データセット",
      locked: "順序固定、無効化や削除はできません",
      foodCount: (n: number) => `${n} 品目`,
      foodAndDishCount: (food: number, dish: number) => `食品 ${food} 品目 · 料理 ${dish} 品`,
      rename: "名前を変更",
      replace: "データを置き換え",
      importTitle: "食品データをインポート",
      replaceTitle: (name: string) => `データソースを置き換え：${name}`,
      nameLabel: "データソース名",
      countImportableFoods: (n: number) => `インポート可能な食品 ${n}`,
      countImportableDishes: (n: number) => `インポート可能な料理 ${n}`,
      countSkipped: (n: number) => `スキップ ${n}`,
      countWarnings: (n: number) => `警告 ${n}`,
      replacesEstimates: (n: number) => `${n} 品目の食品が AI 推定を上書きします`,
      shadowedByManual: (n: number) => `${n} 品目の食品はすでに手動入力で上書きされています`,
      dishShadowedByManual: (n: number) => `${n} 品の料理はすでに手動入力で上書きされています`,
      brandsLabel: (list: string) => `ブランド：${list}`,
      showIssues: "エラーと警告を表示",
      hideIssues: "エラーと警告を隠す",
      colLine: "行",
      colName: "名前",
      colProblem: "問題",
      colProtein: "たんぱく質",
      colCarbs: "炭水化物",
      colFat: "脂質",
      colFiber: "食物繊維",
      colBasis: "基準",
      basisFood: "100g あたり",
      basisDish: "1 食あたり",
      sampleTitle: "例",
      importAction: "インポート",
      deleteConfirm: "削除しても記録済みの食事には影響しません",
    },
  },
  // Members page (`crates/plugin-multimember`).
  members: {
    title: "メンバー",
    intro:
      "一緒に食べる人（16〜70歳）。一緒に食べる食事は全員分を同じ献立で作り、それぞれのアレルギーと苦手な食べ物を避け、体重を保つのに必要な量で一人ずつ盛り付け量を決めます。",
    noMembers: "まだメンバーがいません。追加すると、プランが食卓の全員に合わせて作られます。",
    add: "メンバーを追加",
    edit: "編集",
    archive: "削除",
    archiveConfirm: "削除する",
    memberLine: (age: number, sex: string, pattern: string) => `${age}歳 · ${sex} · ${pattern}`,
    allergiesLine: (list: string) => `アレルギー：${list}`,
    form: {
      addTitle: "メンバーを追加",
      editTitle: (name: string) => `${name} を編集`,
      name: "名前",
      weightKg: "体重（kg）",
      weightHint: "体重を保つのに必要な量の目安に使います。変わったら手動で更新してください。",
      ageHint: "16〜70歳",
      invalid: "名前、生年月日、身長、体重を入力してください。",
    },
    attendance: {
      title: "いつも一緒に食べる食事",
      weekdays: ["月", "火", "水", "木", "金", "土", "日"],
      saving: "保存中…",
    },
    week: {
      title: "これから 7 日間",
      hint: "タップするとその日だけ変更できます（出張、飲み会など）。* はその日だけの変更です。",
      today: "今日",
      reset: "いつも通りに戻す",
      notePlaceholder: "メモ（任意）",
    },
    dislikes: {
      title: "苦手な食べ物",
      placeholder: "例：パクチー",
      add: "追加",
      none: "なし",
    },
    shared: {
      with: (names: string, portions: number) => `一緒に：${names} · 計 ${portions} 人前`,
      gramsNote: "食材のグラムは全員分の合計、カロリーはあなた自身の分です",
      guest: (name: string, kcal: number) => `${name} 約 ${kcal} kcal`,
      estimateNote: "それぞれ体重を保つのに必要な量で見積もった盛り付け量（目安）",
    },
  },
  errors: {
    not_configured: "モデルが設定されていません。先に設定でモデルを設定してください。",
    invalid_input: "入力内容に誤りがあります。確認してから再試行してください。",
    not_found: "該当するデータが見つかりません。",
    nothing_to_plan: "プロフィールを入力し、体重を一度記録してからプランを作成してください。",
    plan_busy: "プランを作成中です。完了までお待ちください。",
    internal: "内部エラーが発生しました。もう一度お試しください。",
    llm_auth: "モデルの認証に失敗しました。API Key が正しいか確認してください。",
    llm_rate_limited: "リクエストが多すぎます。しばらくしてから再試行してください。",
    llm_timeout: "リクエストがタイムアウトしました。もう一度お試しください。",
    llm_unavailable: "モデルのサービスで問題が発生しました。もう一度お試しください。",
    token_budget_exceeded: "今日の AI 利用量の上限に達しました。明日もう一度お試しください。",
    action_not_pending: "この操作はすでに処理されたか、期限切れです。",
    height_out_of_range: (d: { got: number }) =>
      `身長は 120〜230 cm の範囲で入力してください（入力値：${d.got.toFixed(1)} cm）。`,
    age_out_of_range: (d: { got: number }) =>
      `年齢は 16〜100 歳の範囲である必要があります（生年月日から計算した年齢：${d.got} 歳）。`,
    meals_per_day_out_of_range: (d: { got: number }) =>
      `1 日の食事回数は 3〜5 回の範囲で入力してください（入力値：${d.got}）。`,
    goal_rate_out_of_range: (d: { got: number }) =>
      `変化量は 0〜1.0 kg/週の範囲で入力してください（入力値：${d.got.toFixed(2)} kg/週）。`,
    goal_target_out_of_range: (d: { got: number }) =>
      `目標体重は 30〜300 kg の範囲で入力してください（入力値：${d.got.toFixed(1)} kg）。`,
    weight_out_of_range: (d: { got: number }) =>
      `体重は 30〜300 kg の範囲で入力してください（入力値：${d.got.toFixed(1)} kg）。`,
    note_empty: "内容を入力してください。",
    note_too_long: (d: { got: number; max: number }) =>
      `内容は ${d.max} 文字以内で入力してください（現在 ${d.got} 文字）。`,
    plan_time_before_day_boundary: (d: { plan_time: string; day_boundary: string }) =>
      `プランの作成時刻（${d.plan_time}）は 1 日の区切りの時刻（${d.day_boundary}）より前にできません。どちらかを調整してください。`,
    instruction_empty: "調整内容を入力してください。どのように調整するかを書いてください。",
    preference_dates_reversed: (d: { valid_from: string; valid_until: string }) =>
      `開始日（${d.valid_from}）は終了日（${d.valid_until}）より後にできません。`,
    member_name_invalid: "名前は1〜40文字で入力してください。",
    member_name_taken: "同じ名前のメンバーがすでにいます。",
    member_age_out_of_range: (d: { got: number }) =>
      `メンバーの年齢は16〜70歳です。この生年月日では${d.got}歳になります。`,
    member_allergies_invalid: "アレルギーは20件まで、各1〜40文字です。",
    too_many_members: (d: { max: number }) => `メンバーはご本人以外に${d.max}人までです。`,
    attendance_invalid: "食事の予定が正しくありません。過去の日付は指定できず、メモは100文字までです。",
    unauthorized: "ログインしてください。",
    invalid_username: "ユーザー名は半角英数字と _ - . で 3〜32 文字にしてください。",
    invalid_password: "パスワードは 8〜128 文字にしてください。",
    too_many_attempts: "試行回数が多すぎます。しばらくしてから再試行してください。",
    too_many_requests: "リクエストが多すぎます。少し待ってから再試行してください。",
    invalid_credentials: "ユーザー名またはパスワードが正しくありません。",
    username_taken: "このユーザー名はすでに使われています。",
    bad_request: "リクエストの内容に誤りがあります。",
    unknown_endpoint: "該当する API が見つかりません。",
  },
  importMessages: {
    file_too_large: (d: { max_mb: number }) => `ファイルが上限の ${d.max_mb} MB を超えています`,
    not_utf8: "ファイルが UTF-8 ではありません。UTF-8 で保存し直してから再試行してください",
    csv_header: (d: { error: string }) => `CSV の見出し行を読み取れませんでした：${d.error}`,
    csv_row: (d: { line: number; error: string }) =>
      `CSV の ${d.line} 行目を読み取れませんでした：${d.error}`,
    json_parse: (d: { error: string }) => `JSON を読み取れませんでした：${d.error}`,
    json_item_not_object: (d: { line: number }) => `${d.line} 番目の項目がオブジェクトではありません`,
    json_missing_rows: "JSON オブジェクトに foods または dishes がありません",
    json_foods_not_array: "foods は配列である必要があります",
    json_dishes_not_array: "dishes は配列である必要があります",
    json_wrong_shape: "JSON は配列か、foods/dishes を含むオブジェクトである必要があります",
    too_many_rows: (d: { max: number }) => `ファイルの行数が上限の ${d.max} 行を超えています`,
    no_valid_rows: "インポートできる有効な行がファイルにありません",
    missing_file_content: "リクエストにファイルの内容がありません。",
    import_expired: "インポートの有効期限が切れました。もう一度ファイルを選んでください",
    source_name_length: "データソース名は 1〜60 文字にしてください",
    not_a_dataset: "置き換えられるのはデータセットのソースだけです",
    source_not_found: "データソースが見つかりません",
    unknown_basis: (d: { value: string }) =>
      `basis 列の値「${d.value}」を認識できません（100g または serving を指定してください）`,
    not_a_number: (d: { field: string }) => `${d.field} が数値ではありません`,
    missing_field: (d: { field: string }) => `必須の項目 ${d.field} がありません`,
    missing_energy: "必須の項目 kcal または kj がありません",
    not_positive: (d: { field: string; got: number }) =>
      `${d.field} は 0 より大きい値にしてください（入力値：${d.got}）`,
    kcal_kj_mismatch: (d: { kcal: number; kcal_from_kj: number }) =>
      `kcal（${d.kcal.toFixed(0)}）と kj から換算した値（${d.kcal_from_kj.toFixed(0)}）が 2% 以上違います。kcal を使います`,
    energy_mismatch: (d: { kcal: number; estimated: number }) =>
      `kcal（${d.kcal.toFixed(0)}）が栄養素から推定したエネルギー（${d.estimated.toFixed(0)}）と大きく違います。値はそのままインポートします`,
    too_many_aliases: (d: { max: number }) =>
      `別名が ${d.max} 個を超えています。超えた分は無視しました`,
    brand_on_food_row:
      "brand 列は料理行（basis=serving）でのみ有効です。この行（100g 基準）では無視しました",
    food_name_empty: "食品名を入力してください",
    food_name_too_long: (d: { max: number }) => `食品名が ${d.max} 文字を超えています`,
    dish_name_empty: "料理名を入力してください",
    dish_name_too_long: (d: { max: number }) => `料理名が ${d.max} 文字を超えています`,
    brand_too_long: (d: { max: number }) => `ブランド名が ${d.max} 文字を超えています`,
    duplicate_food: "ファイル内の前の行と食品名が重複しています",
    duplicate_dish: "ファイル内の前の行と料理（同じブランド・同じ名前）が重複しています",
    duplicate_food_alias: (d: { alias: string }) =>
      `別名「${d.alias}」がファイル内のほかの食品名または別名と重複しているため、無視しました`,
    duplicate_dish_alias: (d: { alias: string }) =>
      `別名「${d.alias}」が同じブランドのほかの料理名または別名と重複しているため、無視しました`,
    trace_values: (d: { rows: number }) =>
      `${d.rows} 行で「Tr/微量」の表記があり、0 として扱いました`,
    unknown_columns: (d: { columns: string[] }) => `不明な列を無視しました：${d.columns.join("、")}`,
  },
  macroViolations: {
    negative: "すべての値は 0 以上の数値にしてください",
    fiber_exceeds_carbs: (d: { fiber_g: number; carbs_g: number }) =>
      `食物繊維（${d.fiber_g}）が炭水化物（${d.carbs_g}）を超えています。炭水化物は食物繊維を含む総量です`,
    macros_exceed_100g: (d: { sum: number }) =>
      `100g あたりのたんぱく質・炭水化物・脂質の合計は 100 以下にしてください（現在 ${d.sum}）`,
    kcal_per_100g_too_high: (d: { got: number }) =>
      `100g あたりのエネルギーは 900 kcal 以下にしてください（現在 ${d.got}）`,
    kcal_per_serving_too_high: (d: { got: number }) =>
      `1 食あたりのエネルギーは 5000 kcal 以下にしてください（現在 ${d.got}）`,
    serving_not_positive: "1 食分の重さは 0 より大きい値にしてください",
    macros_exceed_serving: (d: { sum: number; serving_g: number }) =>
      `たんぱく質・炭水化物・脂質の合計（${d.sum}）が 1 食分の重さ（${d.serving_g}g）を超えています`,
    kcal_exceeds_serving: (d: { kcal: number; serving_g: number }) =>
      `エネルギー（${d.kcal}）が 1 食分の重さ（${d.serving_g}g）の 9 倍を超えています`,
  },
};
