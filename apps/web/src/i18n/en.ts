import type { Strings } from "./index";

/** English UI strings. Same keys as `./zh-CN.ts`. */
export const en: Strings = {
  app: {
    name: "HumanHealthHandler",
  },
  common: {
    birthDate: { year: "Year", month: "Month", day: "Day" },
    next: "Next",
    back: "Back",
    finish: "Finish",
    finishing: "Finishing…",
    save: "Save",
    saving: "Saving…",
    cancel: "Cancel",
    loading: "Loading…",
    optional: "optional",
    delete: "Delete",
    confirmDelete: "Delete",
    retry: "Retry",
    networkError: "Can't reach the server. Check your connection and try again.",
    logout: "Log out",
    listSeparator: ", ",
    withOptional: (label: string) => `${label} (optional)`,
    labelled: (label: string, detail: string) => `${label}: ${detail}`,
    parenthesized: (text: string) => ` (${text})`,
  },
  auth: {
    loginTitle: "Log in",
    registerTitle: "Sign up",
    username: "Username",
    usernameHint: "3–32 letters, digits or _ - . (not case-sensitive)",
    password: "Password",
    passwordHint: "At least 8 characters",
    confirmPassword: "Confirm password",
    passwordMismatch: "The passwords don't match.",
    login: "Log in",
    register: "Sign up",
    submitting: "Please wait…",
    toRegister: "No account yet? Sign up",
    toLogin: "Already have an account? Log in",
    sessionExpired: "Your session has expired. Please log in again.",
  },
  nav: {
    today: "Today",
    chat: "Chat",
    activity: "Assistant log",
    nutrition: "Nutrition",
    stock: "Stock",
    settings: "Settings",
    training: "Training",
    members: "Members",
  },
  guide: {
    open: "Guide",
    skip: "Skip",
    done: "Get started",
    stepCounter: (n: number, total: number) => `${n} / ${total}`,
    steps: [
      {
        title: "Welcome to HumanHealthHandler",
        body: "HHH logs what you eat and how you train, and every day makes a plan for the next one from your goals. Here's a quick look at each tab.",
      },
      {
        title: "Today: your day at a glance",
        body: "Today's meal plan (with a ✓ on meals already logged), today's training plan and the shopping list in one place. It's for viewing; make changes on each feature's tab or in chat.",
      },
      {
        title: "Chat: log things in a sentence",
        body: "Tell the assistant what you ate or did, as you would a friend, e.g. \"ramen and a beer for lunch\". The app works out calories and nutrients itself.\nGot something wrong? Fix or delete it in chat too.",
      },
      {
        title: "Nutrition: meals and tomorrow's plan",
        body: "See today's calories, protein, carbs and fat, delete logged meals and log your weight.\nTomorrow's plan is made automatically at the time you set. You can also make it now, write a request and regenerate, or accept it.",
      },
      {
        title: "Training and Stock",
        body: "The Training tab shows the last 7 days and your training plan; the Stock tab keeps track of the food at home, and each shopping-list item can be marked done or cancelled. Both can be switched on or off in Settings.",
      },
      {
        title: "Settings",
        body: "Change your profile and goal, switch features on or off, set when plans are made, list foods you dislike, and pick the language. The assistant replies in that language.\nThe ? button brings this guide back any time.",
      },
    ],
  },
  mealType: {
    breakfast: "Breakfast",
    lunch: "Lunch",
    dinner: "Dinner",
    snack: "Snack",
  },
  onboarding: {
    stepLabel: (step: number, total: number) => `Step ${step} of ${total}`,
    steps: ["Basics", "Goal"],
    profile: {
      title: "Basics",
      sex: "Sex",
      sexOptions: { male: "Male", female: "Female" },
      birthDate: "Date of birth",
      heightCm: "Height (cm)",
      weightKg: "Current weight (kg)",
      activityLevel: "Activity level",
      activityLevelHints: {
        sedentary: "Sedentary — little exercise, mostly sitting",
        light: "Lightly active — light exercise 1–3 days a week",
        moderate: "Moderately active — moderate exercise 3–5 days a week",
        active: "Active — exercise 6–7 days a week",
        very_active: "Very active — hard training or physical work",
      },
      dietaryPattern: "Diet",
      dietaryPatternOptions: {
        omnivore: "No restrictions",
        vegetarian: "Vegetarian",
        vegan: "Vegan",
        pescatarian: "Pescatarian",
      },
      allergies: "Food allergies",
      allergiesPlaceholder: "Separate with commas, e.g. shrimp, peanuts",
      dislikes: "Foods you dislike",
      dislikesPlaceholder: "Separate with commas, e.g. cilantro",
      mealsPerDay: "Meals per day",
      medicalNotes: "Health notes",
      medicalNotesPlaceholder: "Any relevant health conditions (optional)",
    },
    goal: {
      title: "Goal",
      direction: "Direction",
      directionOptions: {
        lose: "Lose weight",
        maintain: "Maintain",
        gain: "Gain weight",
      },
      targetWeightKg: "Target weight (kg)",
      targetWeightPlaceholder: "Leave empty to set automatically",
      ratePerWeek: "Weekly rate",
      rateOptions: {
        "0.25": "0.25 kg/week",
        "0.5": "0.5 kg/week",
        "0.75": "0.75 kg/week",
      },
      previewTitle: "Estimated daily calorie target",
      previewKcal: (kcal: number) => `${kcal} kcal/day`,
      previewMacros: (protein: number, carbs: number, fat: number) =>
        `Protein ${protein}g · Carbs ${carbs}g · Fat ${fat}g`,
      previewFloorApplied: "Raised to the safe minimum.",
      previewError: "Can't calculate a preview right now. Check the details above.",
    },
    issues: {
      title: "To continue:",
      sexMissing: "Choose your sex",
      birthDateMissing: "Pick your birth date (year, month and day)",
      heightMissing: "Enter your height",
      heightRange: (min: number, max: number) => `Height must be between ${min} and ${max} cm`,
      weightMissing: "Enter your current weight",
      weightRange: (min: number, max: number) => `Weight must be between ${min} and ${max} kg`,
    },
    errors: {
      required: "Required",
      saveFailed: "Couldn't save. Please try again.",
    },
  },
  today: {
    latestWeight: (kg: number) => `Latest weight ${kg} kg`,
    noWeightYet: "No weight logged yet",
    logWeight: "Log weight",
    logWeightTitle: "Log weight",
    logWeightPlaceholder: "kg",
    logWeightInvalid: "Enter a weight above 0 (kg)",
    diet: {
      title: "Today's food",
      kcalEatenOfTarget: (eaten: number, target: number) => `${eaten} / ${target} kcal`,
      kcalEaten: (eaten: number) => `${eaten} kcal`,
      ofTargetKcal: (target: number) => `/ ${target} kcal`,
      noTarget: "No target yet: complete your profile and log your weight",
      protein: "Protein",
      carbs: "Carbs",
      fat: "Fat",
      gramsShort: (g: number) => `${g}g`,
      servingsShort: (n: number) => `×${n}`,
      noMealsYet: "Nothing logged today yet",
      estimatedBadge: "est.",
    },
    todayPlan: {
      title: "Today's plan",
      eaten: "Eaten",
      notYetEaten: "Not yet",
    },
    tomorrowPlan: {
      title: "Tomorrow's plan",
      noPlanYet: (time: string) => `Will be created automatically at ${time}`,
      generateNow: "Create now",
      generating: "Creating…",
      accept: "Accept plan",
      regenerate: "Regenerate",
      adjust: "Adjust",
      adjusting: "Adjusting…",
      adjustPrefill: "Adjust tomorrow's plan: ",
      warningsTitle: "Notes",
      medicalDisclaimer:
        "Your profile records a health condition. This plan is lifestyle advice only; follow your doctor's or dietitian's guidance first.",
      dayTotal: (kcal: number, target: number) => `Day total ${kcal} / ${target} kcal`,
      trainingLine: (title: string, minutes: number) => `Training: ${title} · ${minutes} min`,
      trainingRest: "Training: rest day",
      trainingKcalAdded: (kcal: number) => `+${kcal} kcal added for training`,
      generateFailed: "Couldn't create the plan. Please try again.",
      requestLabel: "Requests for this plan",
      requestPlaceholder: "Requests for this plan (optional), e.g. working late tomorrow, convenience-store food only",
      requestPlaceholderWithPlan:
        "Requests (optional), e.g. working late tomorrow, convenience-store food only. Regenerate makes a new plan with them; Adjust changes the current one.",
    },
    deleteMealConfirm: "Delete this entry?",
  },
  overview: {
    dietTitle: "Today's meal plan",
    noDietPlan: "No meal plan for today. You can make tomorrow's on the Nutrition tab.",
    noTrainingPlan: "No training plan for today.",
  },
  stock: {
    hint: "You can also tell the assistant in chat, e.g. \"bought a dozen eggs and 1 L of milk\".",
    autoConsume: "Take logged meals out of stock automatically",
    planFromStock: "Plan from stock and make a shopping list",
    itemsTitle: "In stock",
    empty: "Nothing in stock",
    units: { g: "g", ml: "ml", piece: "pcs" },
    amount: (n: number, unit: string) => `${n} ${unit}`,
    bestBefore: (d: string) => `best before ${d}`,
    expired: "expired",
    quantityLabel: (name: string) => `${name} left`,
    add: "Add",
    adding: "Adding…",
    name: "Name",
    namePlaceholder: "e.g. eggs",
    quantity: "Quantity",
    unit: "Unit",
    expiresOn: "Best before",
    shoppingTitle: "Shopping list",
    shoppingFor: (d: string) => `From the plan for ${d}`,
    noShopping: "No plan yet. Once tomorrow's plan is made, what to buy shows up here.",
    nothingToBuy: "Your stock covers the plan; nothing to buy.",
    inStock: (amount: string) => `${amount} in stock`,
    complete: "Done",
    completeHint: "Bought it: adds the planned amount to stock",
    cancel: "Cancel",
    reopen: "Restore",
    statusDone: "Bought",
    statusCancelled: "Cancelled",
    recentTitle: "Recently used",
    recentEmpty: "Stock used by logged meals shows up here.",
    usedToast: "The meal you just logged took this from stock:",
    undo: "Undo",
    undoing: "Undoing…",
    undoHint: "Put it back in stock; the meal stays logged",
    dismiss: "Close",
  },
  training: {
    title: "Training",
    week: {
      title: "Last 7 days",
      summary: (sessions: number, minutes: number) =>
        `${sessions} ${sessions === 1 ? "session" : "sessions"} · ${minutes} min`,
      kcal: (kcal: number) => `about ${kcal} kcal burned`,
      kcalHint: "Exercise only (resting burn excluded), estimated by the app from exercise, time and weight",
      empty: "No training logged in the last 7 days. Tell me in chat what you did, e.g. \"ran 5 km in 30 minutes today\".",
      today: "Today",
      rest: "—",
      chatHint: "Log training in chat, e.g. \"just did squats, 4 sets of 8 at 60 kg\".",
    },
    todayPlan: {
      title: "Today's training plan",
    },
    tomorrow: {
      title: (date: string) => `Tomorrow's training plan (${date})`,
      noPlan: "No plan for tomorrow yet.",
      noTrainingSection: "Tomorrow's plan was made before training was turned on. Regenerate it to add training.",
      preferenceLabel: "How would you like to train tomorrow? (optional)",
      preferencePlaceholder: "e.g. legs, 45 min; a morning run; or a rest day",
      savePreference: "Save",
      preferenceSaved: "Saved",
      generate: "Create tomorrow's plan",
      regenerate: "Regenerate with this",
      generating: "Generating…",
      dietNote: "Tomorrow's meals are planned around this training plan.",
    },
    plan: {
      restDay: "Rest day",
      untitled: "Training",
      minutes: (minutes: number) => `${minutes} min`,
      kcal: (kcal: number) => `about ${kcal} kcal`,
      timeOfDay: {
        morning: "Morning",
        midday: "Midday",
        afternoon: "Afternoon",
        evening: "Evening",
      },
      intensity: {
        low: "Low intensity",
        moderate: "Moderate intensity",
        high: "High intensity",
      },
    },
    exercise: {
      setsReps: (sets: number, reps: number) => `${sets}×${reps}`,
      sets: (sets: number) => `${sets} sets`,
      reps: (reps: number) => `${reps} reps`,
      weight: (kg: number) => `${kg} kg`,
      duration: (minutes: number) => `${minutes} min`,
      distance: (km: number) => `${km} km`,
    },
    preferences: {
      title: "Training preferences",
      goal: "Goal",
      goalOptions: {
        general_fitness: "General fitness",
        strength: "Strength",
        muscle: "Muscle gain",
        endurance: "Endurance",
        fat_loss: "Fat loss",
      },
      daysPerWeek: "Training days per week",
      sessionMinutes: "Minutes per session",
      equipment: "Equipment / place",
      equipmentPlaceholder: "e.g. gym; or home with dumbbells",
      notes: "Notes",
      notesPlaceholder: "e.g. old left knee injury, avoid jumping",
      invalid: "Days per week must be 1–7 and minutes per session 10–180.",
    },
  },
  chat: {
    inputPlaceholder: "Type a message (Enter to send, Shift+Enter for a new line)",
    readOnly: "Ask without changing records",
    readOnlyHint: "For translations, explanations and calculations. Health data, meal records and plans stay unchanged. The conversation is still saved.",
    send: "Send",
    sending: "Sending…",
    empty: "No messages yet. Say something!",
    thinking: "Thinking",
    deepThinking: "Thinking hard",
    sendFailed: "Couldn't send",
    notices: {
      empty_reply: "The model didn't return a reply. Please try again.",
      too_many_steps: "Stopped after too many steps. What got done is listed below; send the rest again.",
    },
  },
  approvals: {
    title: "Needs your approval",
    hint: "The assistant asked to make these changes. Nothing happens until you approve.",
    approve: "Approve",
    reject: "Reject",
    approved: "Approved and done",
    rejected: "Rejected, nothing changed",
    expired: "Expired, nothing changed",
    failed: (reason: string) => `Approved, but it failed: ${reason}`,
    failedToResolve: "Couldn't do that",
    previews: {
      update_allergies: (d: { add: string[]; remove: string[] }) =>
        [
          d.add.length > 0 ? `Add allergies: ${d.add.join(", ")}` : "",
          d.remove.length > 0 ? `Remove allergies: ${d.remove.join(", ")}` : "",
        ]
          .filter((s) => s !== "")
          .join("; "),
      add_preference: (d: {
        preference_kind: "dislike" | "general";
        text: string;
        valid_from: string | null;
        valid_until: string | null;
        permanent: boolean;
      }) =>
        `Remember ${d.preference_kind === "dislike" ? "a disliked food" : "a preference"}: ${d.text}` +
        (d.permanent
          ? " (always)"
          : d.valid_from || d.valid_until
            ? ` (${d.valid_from ?? ""} to ${d.valid_until ?? ""})`
            : " (one month)"),
      remove_preference: (d: { preference_kind: "dislike" | "general"; text: string }) =>
        `Delete ${d.preference_kind === "dislike" ? "a disliked food" : "a preference"}: ${d.text}`,
      delete_past_entry: (d: { logical_date: string; description: string | null }) =>
        `Delete the ${d.logical_date} record${d.description ? `: ${d.description}` : ""}`,
      archive_member: (d: { name: string }) => `Remove member: ${d.name}`,
      update_member_allergies: (d: { name: string; allergies: string[] }) =>
        `Change ${d.name}'s allergies to: ${d.allergies.length > 0 ? d.allergies.join(", ") : "none"}`,
      add_member: (d: { name: string; allergies: string[] }) =>
        `Add member: ${d.name} (allergies: ${d.allergies.length > 0 ? d.allergies.join(", ") : "none"})`,
      update_training_notes: (d: { notes: string; previous_notes: string; other_changes: boolean }) =>
        `Change training notes (injuries etc.) to: ${d.notes || "none"}` +
        (d.previous_notes ? ` (was: ${d.previous_notes})` : "") +
        (d.other_changes ? ", and other training settings" : ""),
      update_nutrition: (d) =>
        `Change nutrition of ${d.brand ? `${d.brand} ` : ""}${d.name} (${d.basis === "per_100g" ? "per 100 g" : "per serving"}): ` +
        `${d.before ? `${d.before.kcal} → ` : ""}${d.after.kcal} kcal · ` +
        `Protein ${d.after.protein_g}g · Carbs ${d.after.carbs_g}g · Fat ${d.after.fat_g}g` +
        d.reasons
          .map((r) => (r === "large_change" ? " (large change)" : " (kcal doesn't match the macros)"))
          .join(""),
      add_food_alias: (d: { alias: string; food: string }) => `Log "${d.alias}" as "${d.food}"`,
      remove_food_alias: (d: { alias: string; target: string }) =>
        `Stop logging "${d.alias}" as "${d.target}"`,
    },
    unknown: (tool: string) => `Run: ${tool}`,
  },
  activity: {
    hint: "Everything the assistant did for you is recorded here: who started it, what it called, and what was blocked or is waiting for your approval.",
    period: (days: number) => `Last ${days} days`,
    stats: {
      byUser: "Started by you",
      byScheduler: "Automatic",
      approvals: "Approval requests",
      approvalsDetail: (approved: number, rejected: number) =>
        `${approved} approved · ${rejected} rejected`,
      guardrails: "Blocked by guardrails",
      tokens: "Model tokens",
      last24h: (used: number, budget: number | null) =>
        budget === null ? `${used} in the last 24 h` : `${used} of ${budget} in the last 24 h`,
    },
    filterAll: "All",
    actors: { user: "You", scheduler: "Automatic" },
    triggers: {
      chat_turn: "Chat",
      plan_scheduled: "Scheduled plan",
      plan_manual: "Make plan",
      plan_revision: "Adjust plan",
      action_approved: "Approved an action",
      action_rejected: "Rejected an action",
    },
    statuses: {
      running: "Running",
      succeeded: "Done",
      needs_approval: "Needs approval",
      failed: "Failed",
      interrupted: "Interrupted",
    },
    counts: (o: { model_calls: number; tool_calls: number; tokens: number; guardrails: number }) =>
      [
        `${o.model_calls} model ${o.model_calls === 1 ? "call" : "calls"}`,
        `${o.tool_calls} tool ${o.tool_calls === 1 ? "call" : "calls"}`,
        `${o.tokens} tokens`,
        o.guardrails > 0 ? `${o.guardrails} blocked` : "",
      ]
        .filter((s) => s !== "")
        .join(" · "),
    planSaved: (date: string, version: number) => `Plan for ${date}, version ${version}`,
    errorCodes: {
      invalid_input: "Invalid input, or nothing to plan",
      internal: "Internal error",
      budget_exceeded: "Daily token limit reached",
      busy: "Another plan is being made",
      plan_blocked: "Plan failed a safety check and was not saved",
      not_configured: "No model configured",
      action_not_pending: "Already handled or expired",
      rate_limited: "The model is busy",
    } as Record<string, string>,
    errorCode: (code: string) => `Error: ${code}`,
    empty: "Nothing recorded yet.",
    loadMore: "Load more",
    showSteps: "Show steps",
    hideSteps: "Hide",
    roles: { conversation: "conversation", parser: "parsing", planner: "planning" },
    modelCall: (role: string, model: string, tokens: number, seconds: string, ok: boolean) =>
      `Model call (${role}) ${model} · ${tokens} tokens · ${seconds} s${ok ? "" : " · failed"}`,
    toolOutcomes: {
      ran: "done",
      invalid_args: "refused",
      read_only_mode: "not run: read-only question",
      not_found: "not found",
      failed: "error",
    },
    arguments: "Arguments",
    approvalRequested: "Asked for your approval",
    approvalResolved: {
      approved: "You approved",
      rejected: "You rejected",
      failed: "Approved, but it failed",
      pending: "Waiting",
      expired: "Expired",
    },
    fromEarlierRun: "(from an earlier chat)",
    actionStatus: {
      pending: "Still waiting for you",
      approved: "Approved",
      rejected: "Rejected",
      failed: "Failed after approval",
      expired: "Expired, not done",
    },
    guardrails: {
      invalid_args: "Blocked: arguments or rules don't allow it",
      read_only_mode: "Blocked: this question cannot change records",
      unknown_tool: "Blocked: no such tool, or it's switched off",
      token_budget: "Blocked: over the daily token limit",
      iteration_cap: "Stopped: too many steps",
    },
    planValidation: (d: { violations: readonly unknown[]; retried: boolean; violations_left: number; blocking_violations_left?: number }) =>
      (d.blocking_violations_left ?? 0) > 0
        ? `Plan check: ${d.blocking_violations_left} safety issues remain; saving was blocked`
        : d.violations.length === 0
        ? "Plan check: passed"
        : d.violations_left === 0
          ? `Plan check: ${d.violations.length} issues found, all fixed on a retry`
          : `Plan check: ${d.violations.length} issues found, ${d.violations_left} left (kept as warnings)`,
    planSavedStep: (date: string, version: number) => `Saved the plan for ${date} (version ${version})`,
  },
  planNotes: {
    unmet: (text: string) => `Not fully met: ${text}`,
    core_section_missing: (d: { section: string }) => `The plan has no "${d.section}" section`,
    diet_invalid_structure: "The meal plan has an invalid format",
    diet_item_unnamed: "A food item has no name",
    diet_grams_out_of_range: (d: { name: string; grams: number; min: number; max: number }) =>
      `"${d.name}" is ${d.grams} g, outside ${d.min}–${d.max} g`,
    diet_allergen: (d: {
      meal_type: "breakfast" | "lunch" | "dinner" | "snack";
      meal_title: string;
      name: string;
      allergen: string;
    }) =>
      `"${d.name}" in ${{ breakfast: "breakfast", lunch: "lunch", dinner: "dinner", snack: "snack" }[d.meal_type]} "${d.meal_title}" contains the allergen "${d.allergen}"`,
    diet_disliked: (d: { name: string; term: string }) =>
      `"${d.name}" contains "${d.term}", which you dislike`,
    diet_main_meal_count: (d: { count: number }) =>
      `There should be 3 main meals (breakfast, lunch, dinner), not ${d.count}`,
    diet_meal_count: (d: { count: number; max: number }) =>
      `There should be 3–${d.max} meals in total, not ${d.count}`,
    diet_kcal_off_target: (d: { actual: number; target: number; low: number; high: number }) =>
      `The day's ${d.actual} kcal is outside ±8% of the ${d.target} kcal target (${d.low}–${d.high})`,
    diet_protein_low: (d: { actual: number; target: number }) =>
      `Protein ${d.actual} g is below 90% of the ${d.target} g target`,
    diet_fat_off_target: (d: { actual: number; target: number }) =>
      `Fat ${d.actual} g is more than 20% off the ${d.target} g target`,
    diet_carbs_off_target: (d: { actual: number; target: number }) =>
      `Carbs ${d.actual} g are more than 20% off the ${d.target} g target`,
    diet_fiber_low: (d: { actual: number; target: number }) =>
      `Fiber ${d.actual} g is below 70% of the ${d.target} g target`,
    diet_guest_allergen: (d: {
      meal_type: "breakfast" | "lunch" | "dinner" | "snack";
      meal_title: string;
      name: string;
      guest: string;
      allergen: string;
    }) =>
      `"${d.name}" in ${{ breakfast: "breakfast", lunch: "lunch", dinner: "dinner", snack: "snack" }[d.meal_type]} "${d.meal_title}" contains ${d.guest}'s allergen "${d.allergen}"`,
    diet_guest_disliked: (d: { name: string; guest: string; term: string }) =>
      `"${d.name}" contains "${d.term}", which ${d.guest} dislikes`,
    diet_shared_pattern: (d: {
      meal_type: "breakfast" | "lunch" | "dinner" | "snack";
      meal_title: string;
      pattern: "omnivore" | "vegetarian" | "vegan" | "pescatarian";
      name: string;
      term: string;
    }) =>
      `${{ breakfast: "Breakfast", lunch: "Lunch", dinner: "Dinner", snack: "Snack" }[d.meal_type]} "${d.meal_title}" is shared and must be ${{ omnivore: "omnivore", vegetarian: "vegetarian", vegan: "vegan", pescatarian: "pescatarian" }[d.pattern]}: "${d.name}" contains "${d.term}"`,
    diet_guest_scale_clamped: (d: { guest: string; scale: number; min: number; max: number }) =>
      `${d.guest}'s portion scale was limited to ${d.min}–${d.max} (${d.scale})`,
    stock_invalid_structure: "The stock plan has an invalid format",
    stock_not_in_stock: (d: { name: string }) => `"${d.name}" is not in stock, so the plan can't use it`,
    stock_wrong_unit: (d: { name: string; unit: "g" | "ml" | "piece"; planned_unit: "g" | "ml" | "piece" }) =>
      `"${d.name}" is stocked in ${{ g: "g", ml: "ml", piece: "pieces" }[d.unit]}, not ${{ g: "g", ml: "ml", piece: "pieces" }[d.planned_unit]}`,
    stock_over_quantity: (d: { name: string; amount: number; quantity: number; unit: "g" | "ml" | "piece" }) =>
      `"${d.name}" uses ${d.amount} ${{ g: "g", ml: "ml", piece: "pieces" }[d.unit]}, but only ${d.quantity} ${{ g: "g", ml: "ml", piece: "pieces" }[d.unit]} is in stock`,
    stock_expiring_unused: (d: { name: string; expires_on: string }) =>
      `"${d.name}" in stock expires on ${d.expires_on} and the plan doesn't use it`,
    training_invalid_structure: "The training plan has an invalid format",
    training_invalid_exercise: (d: {
      name: string;
      problem: "name_empty" | "name_too_long" | "sets" | "reps" | "weight" | "duration" | "distance";
    }) =>
      `"${d.name}": ${
        {
          name_empty: "the name is empty",
          name_too_long: "the name is too long",
          sets: "sets must be 1–20",
          reps: "reps must be 1–200",
          weight: "weight must be 0–500 kg",
          duration: "duration must be 0–300 minutes",
          distance: "distance must be 0–300 km",
        }[d.problem]
      }`,
    training_rest_day_too_hard: "A rest day may only have low-intensity stretching or mobility work",
    training_no_exercises: "A training day needs at least one exercise",
    training_too_long: (d: { minutes: number; max: number }) =>
      `The session's ${d.minutes} minutes exceed the ${d.max}-minute limit`,
    training_over_usual: (d: { minutes: number; usual: number }) =>
      `The session's ${d.minutes} minutes are well over your usual ${d.usual}`,
    training_hard_after_break: "No training in the last 7 days; start at low to moderate intensity",
    training_big_jump: (d: { longest: number }) =>
      `Over 1.5× your longest session of the last 7 days (${d.longest} minutes); build up gradually`,
    training_third_hard_day: "The previous two days were high intensity; go easier or rest",
    multimember_out_of_age_range: (d: { name: string; age: number; min: number; max: number }) =>
      `${d.name} is ${d.age}, outside ${d.min}–${d.max}, and was left out of shared meals`,
  },
  toolSummaries: {
    core_plan_viewed: (d: { date: string }) => `Viewed the plan for ${d.date}`,
    core_plan_generated: (d: { date: string; version: number }) =>
      `Made the plan for ${d.date} (version ${d.version})`,
    core_plan_revised: (d: { date: string; version: number }) =>
      `Adjusted the plan for ${d.date} (version ${d.version})`,
    core_weight_logged: (d: { kg: number }) => `Logged weight: ${d.kg} kg`,
    core_allergies_updated: (d: { allergies: string[] }) =>
      `Updated allergies: ${d.allergies.length > 0 ? d.allergies.join(", ") : "none"}`,
    core_preference_added: (d: {
      preference_kind: "dislike" | "general";
      text: string;
      valid_from: string | null;
      valid_until: string | null;
    }) =>
      `Remembered ${d.preference_kind === "dislike" ? "a dislike" : "a preference"}: ${d.text}` +
      (d.valid_until ? ` (until ${d.valid_until})` : ""),
    core_preference_removed: "Removed a preference",
    core_entry_deleted: "Deleted an entry",
    diet_meal_logged: (d: { meal_type: "breakfast" | "lunch" | "dinner" | "snack"; kcal: number }) =>
      `Logged ${{ breakfast: "breakfast", lunch: "lunch", dinner: "dinner", snack: "a snack" }[d.meal_type]} · ${d.kcal} kcal`,
    diet_intake_viewed: (d: { date: string }) => `Viewed meals for ${d.date}`,
    diet_label_calculated: (d: { portion_grams: number }) => `Calculated label nutrition for ${d.portion_grams} g; no intake recorded`,
    diet_food_updated: (d: { name: string }) => `Updated food: ${d.name}`,
    diet_dish_updated: (d: { name: string }) => `Updated dish: ${d.name}`,
    diet_alias_added: (d: { alias: string; food: string }) => `Added alias: ${d.alias} → ${d.food}`,
    diet_alias_removed: (d: { alias: string }) => `Removed alias: ${d.alias}`,
    stock_added: (d: { names: string[] }) => `Added to stock: ${d.names.join(", ")}`,
    stock_viewed: "Viewed stock",
    stock_updated: (d: { name: string }) => `Updated stock: ${d.name}`,
    training_session_logged: (d: { title: string; minutes: number }) =>
      `Logged training: ${d.title}, ${d.minutes} min`,
    training_log_viewed: (d: { start_date: string; end_date: string }) =>
      `Viewed training from ${d.start_date} to ${d.end_date}`,
    training_day_preference_set: (d: { date: string; cleared: boolean }) =>
      d.cleared
        ? `Cleared the training wish for ${d.date}`
        : `Saved the training wish for ${d.date}`,
    training_preferences_updated: "Updated training preferences",
    multimember_members_viewed: (d: { count: number }) => `Viewed members (${d.count})`,
    multimember_member_added: (d: { name: string }) => `Added member: ${d.name}`,
    multimember_member_updated: (d: { name: string }) => `Updated member: ${d.name}`,
    multimember_member_archived: (d: { name: string }) => `Removed member: ${d.name}`,
    multimember_attendance_set: (d: {
      name: string;
      meal_type: "breakfast" | "lunch" | "dinner";
      present: boolean;
      date: string | null;
    }) =>
      d.date
        ? `${d.name} ${d.present ? "joins" : "skips"} ${d.meal_type} on ${d.date}`
        : `${d.name} usually ${d.present ? "joins" : "skips"} ${d.meal_type}`,
    multimember_attendance_cleared: (d: {
      name: string;
      meal_type: "breakfast" | "lunch" | "dinner";
      date: string;
    }) => `${d.name}: back to the usual ${d.meal_type} on ${d.date}`,
    multimember_dislike_added: (d: { name: string; text: string }) =>
      `Remembered ${d.name} dislikes ${d.text}`,
    multimember_dislike_removed: "Removed a dislike",
  },
  tools: {
    core_get_plan: "View plan",
    core_generate_plan: "Make plan",
    core_revise_plan: "Adjust plan",
    core_log_weight: "Log weight",
    core_update_allergies: "Update allergies",
    core_add_preference: "Remember a preference",
    core_remove_preference: "Remove a preference",
    core_delete_entry: "Delete an entry",
    diet_log_meal: "Log a meal",
    diet_calculate_label: "Calculate label nutrition",
    diet_get_intake: "View meals",
    diet_update_food: "Update food",
    diet_update_dish: "Update dish",
    diet_add_alias: "Add alias",
    diet_remove_alias: "Remove alias",
    stock_add: "Add to stock",
    stock_list: "View stock",
    stock_update: "Update stock",
    training_log_session: "Log training",
    training_get_log: "View training",
    training_set_day_preference: "Set a day's training wish",
    training_update_preferences: "Update training preferences",
    multimember_list_members: "View members",
    multimember_add_member: "Add member",
    multimember_update_member: "Update member",
    multimember_archive_member: "Remove member",
    multimember_set_attendance: "Set meal attendance",
    multimember_clear_attendance_override: "Reset attendance to usual",
    multimember_add_dislike: "Remember a member's dislike",
    multimember_remove_dislike: "Remove a member's dislike",
  } as Record<string, string>,
  settings: {
    saveSucceeded: "Saved",
    saveFailed: "Couldn't save. Please try again.",
    sections: {
      profileGoal: "Profile and goal",
      foodData: "Food data",
      plan: "Plan",
      preferences: "Preferences",
      account: "Account",
      language: "Language",
      features: "Features",
    },
    features: {
      hint: "Optional features appear in the sidebar and join the daily plan once turned on.",
      activityTab: "Show the Assistant log page",
      activityTabHint: "The assistant's actions are always recorded; this only decides whether the page shows in the sidebar.",
      names: {
        training: "Training log & plan",
        stock: "Stock",
        multimember: "Shared meals",
      } as Record<string, string>,
      descriptions: {
        training:
          "Log workouts in chat and get a daily training plan from your preferences and the last 7 days. The next day's meals are planned around the training.",
        stock:
          "Keep track of the food at home. Logged meals take what they used out of stock, and plans use what you have first and come with a shopping list.",
        multimember:
          "Add the people you share meals with (aged 16–70). Shared meals become one set of dishes that avoids everyone's allergies and dislikes, with a portion for each person sized to keep their weight.",
      } as Record<string, string>,
    },
    language: {
      hint: "The page reloads after switching.",
    },
    account: {
      loggedInAs: (username: string) => `Logged in as ${username}`,
      deleteTitle: "Delete account",
      deleteHint:
        "Permanently deletes this account and all its data (food log, plans, chats, preferences). This can't be undone.",
      passwordToConfirm: "Enter your password to confirm",
      deleteButton: "Delete account permanently",
      deleting: "Deleting…",
    },
    plan: {
      planTime: "Plan creation time",
      dayBoundary: "Day starts at",
    },
    preferences: {
      hint: "Foods you dislike and anything else your plans should follow. What you tell the assistant in chat is saved here too. Each one applies only for its dates.",
      empty: "Nothing saved yet",
      kinds: { dislike: "Dislike", general: "Wish" },
      kind: "Type",
      text: "Details",
      dislikePlaceholder: "e.g. cilantro",
      generalPlaceholder: "e.g. weekday lunch is at the company canteen",
      validFrom: "From",
      validUntil: "Until",
      permanent: "No end date",
      validityHint: "Without an end date, it lasts one month from the start date (or from today).",
      add: "Add",
      adding: "Adding…",
      from: (d: string) => `From ${d}`,
      until: (d: string) => `Until ${d}`,
      between: (from: string, until: string) => `${from} – ${until}`,
      expired: "Expired",
    },
    foodData: {
      hint: "Sources are used from highest to lowest priority; the first match wins.",
      importButton: "Import data…",
      formatHelp:
        "You can import a food composition table as CSV or JSON, optionally with an aliases column. Energy is read as kcal by default; for kJ, name the column kj or energy_kj. You can also import per-serving dishes such as chain restaurant menus: a basis column tells 100g food rows from serving dish rows, and dish rows can name the chain in a brand column. On dish rows, basis_g is the weight of one serving in grams.",
      empty:
        "No datasets imported yet. You can import a food composition table as CSV or JSON, optionally with an aliases column. Energy is read as kcal by default; for kJ, name the column kj or energy_kj.",
      kindManual: "Manual entries",
      kindEstimate: "AI estimates",
      kindDataset: (date: string) => `Dataset · imported ${date}`,
      kindDatasetPlain: "Dataset",
      locked: "Fixed position; can't be disabled or deleted",
      foodCount: (n: number) => `${n} foods`,
      foodAndDishCount: (food: number, dish: number) => `${food} foods · ${dish} dishes`,
      rename: "Rename",
      replace: "Replace data",
      importTitle: "Import food data",
      replaceTitle: (name: string) => `Replace source: ${name}`,
      nameLabel: "Source name",
      countImportableFoods: (n: number) => `Foods to import ${n}`,
      countImportableDishes: (n: number) => `Dishes to import ${n}`,
      countSkipped: (n: number) => `Skipped ${n}`,
      countWarnings: (n: number) => `Warnings ${n}`,
      replacesEstimates: (n: number) => `${n} foods will replace AI estimates`,
      shadowedByManual: (n: number) => `${n} foods are already overridden by manual entries`,
      dishShadowedByManual: (n: number) => `${n} dishes are already overridden by manual entries`,
      brandsLabel: (list: string) => `Brands: ${list}`,
      showIssues: "Show errors and warnings",
      hideIssues: "Hide errors and warnings",
      colLine: "Line",
      colName: "Name",
      colProblem: "Problem",
      colProtein: "Protein",
      colCarbs: "Carbs",
      colFat: "Fat",
      colFiber: "Fiber",
      colBasis: "Basis",
      basisFood: "per 100g",
      basisDish: "per serving",
      sampleTitle: "Sample",
      importAction: "Import",
      deleteConfirm: "Meals already logged are not affected",
    },
  },
  // Members page (`crates/plugin-multimember`).
  members: {
    title: "Members",
    intro:
      "The people who eat with you (aged 16–70). Shared meals are planned as one set of dishes that avoids everyone's allergies and dislikes, with a portion for each person sized to keep their weight.",
    noMembers: "No members yet. Once added, plans take everyone at the table into account.",
    add: "Add member",
    edit: "Edit",
    archive: "Remove",
    archiveConfirm: "Confirm remove",
    memberLine: (age: number, sex: string, pattern: string) => `${age} · ${sex} · ${pattern}`,
    allergiesLine: (list: string) => `Allergies: ${list}`,
    form: {
      addTitle: "Add member",
      editTitle: (name: string) => `Edit ${name}`,
      name: "Name",
      weightKg: "Weight (kg)",
      weightHint: "Used to estimate what keeps their weight; update it by hand when it changes.",
      ageHint: "Aged 16–70",
      invalid: "Enter a name, birth date, height and weight.",
    },
    attendance: {
      title: "Usual shared meals",
      weekdays: ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"],
      saving: "Saving…",
    },
    week: {
      title: "Next 7 days",
      hint: "Tap a meal to change just that day (a trip, a dinner out). * marks a one-off change.",
      today: "Today",
      reset: "Back to usual",
      notePlaceholder: "Note (optional)",
    },
    dislikes: {
      title: "Dislikes",
      placeholder: "e.g. cilantro",
      add: "Add",
      none: "None",
    },
    shared: {
      with: (names: string, portions: number) => `With ${names} · ${portions} portions`,
      gramsNote: "Ingredient grams are for everyone; kcal is your own portion",
      guest: (name: string, kcal: number) => `${name} about ${kcal} kcal`,
      estimateNote: "Portions estimated to keep each person's weight",
    },
  },
  errors: {
    not_configured: "No model is set up yet. Set one up in Settings first.",
    invalid_input: "Something in the input isn't valid. Check it and try again.",
    not_found: "That item couldn't be found.",
    nothing_to_plan: "Complete your profile and log your weight once, then create a plan.",
    plan_busy: "A plan is already being created. Please wait for it to finish.",
    plan_blocked: "The generated plan failed a safety check and was not saved. Existing plans are unchanged. Adjust your request and try again.",
    internal: "Something went wrong. Please try again.",
    llm_auth: "The model rejected the API key. Check that it's correct.",
    llm_rate_limited: "Too many requests. Please wait a moment and try again.",
    llm_timeout: "The request timed out. Please try again.",
    llm_unavailable: "The model service ran into a problem. Please try again.",
    token_budget_exceeded: "You've reached today's AI usage limit. Please try again tomorrow.",
    action_not_pending: "That was already handled or has expired.",
    height_out_of_range: (d: { got: number }) =>
      `Height must be 120–230 cm (entered: ${d.got.toFixed(1)} cm).`,
    age_out_of_range: (d: { got: number }) =>
      `Age must be 16–100 (the date of birth gives ${d.got}).`,
    meals_per_day_out_of_range: (d: { got: number }) =>
      `Meals per day must be 3–5 (entered: ${d.got}).`,
    goal_rate_out_of_range: (d: { got: number }) =>
      `The weekly rate must be 0–1.0 kg/week (entered: ${d.got.toFixed(2)} kg/week).`,
    goal_target_out_of_range: (d: { got: number }) =>
      `Target weight must be 30–300 kg (entered: ${d.got.toFixed(1)} kg).`,
    weight_out_of_range: (d: { got: number }) =>
      `Weight must be 30–300 kg (entered: ${d.got.toFixed(1)} kg).`,
    note_empty: "Enter some text.",
    note_too_long: (d: { got: number; max: number }) =>
      `This can be at most ${d.max} characters (it has ${d.got}).`,
    plan_time_before_day_boundary: (d: { plan_time: string; day_boundary: string }) =>
      `The plan creation time (${d.plan_time}) can't be earlier than the start of the day (${d.day_boundary}). Change one of them.`,
    instruction_empty: "Describe how the plan should be adjusted.",
    preference_dates_reversed: (d: { valid_from: string; valid_until: string }) =>
      `The start date (${d.valid_from}) can't be after the end date (${d.valid_until}).`,
    member_name_invalid: "Name must be 1–40 characters.",
    member_name_taken: "Another member already has this name.",
    member_age_out_of_range: (d: { got: number }) =>
      `Members must be aged 16–70; this birth date gives ${d.got}.`,
    member_allergies_invalid: "Up to 20 allergies, each 1–40 characters.",
    too_many_members: (d: { max: number }) => `Up to ${d.max} members besides you.`,
    attendance_invalid: "Invalid attendance: the date can't be in the past, and notes are up to 100 characters.",
    unauthorized: "Please log in.",
    invalid_username: "Usernames are 3–32 letters, digits or _ - .",
    invalid_password: "Passwords are 8–128 characters.",
    too_many_attempts: "Too many attempts. Please try again later.",
    too_many_requests: "Too many requests. Please wait a moment and try again.",
    invalid_credentials: "Wrong username or password.",
    username_taken: "That username is already taken.",
    bad_request: "The request wasn't valid.",
    unknown_endpoint: "That API endpoint doesn't exist.",
  },
  importMessages: {
    file_too_large: (d: { max_mb: number }) => `The file is larger than ${d.max_mb} MB`,
    not_utf8: "The file isn't UTF-8. Save it as UTF-8 and try again",
    csv_header: (d: { error: string }) => `Couldn't read the CSV header: ${d.error}`,
    csv_row: (d: { line: number; error: string }) =>
      `Couldn't read CSV line ${d.line}: ${d.error}`,
    json_parse: (d: { error: string }) => `Couldn't read the JSON: ${d.error}`,
    json_item_not_object: (d: { line: number }) => `Item ${d.line} isn't an object`,
    json_missing_rows: "The JSON object has no foods or dishes field",
    json_foods_not_array: "foods must be an array",
    json_dishes_not_array: "dishes must be an array",
    json_wrong_shape: "The JSON must be an array, or an object with foods/dishes fields",
    too_many_rows: (d: { max: number }) => `The file has more than ${d.max} rows`,
    no_valid_rows: "The file has no valid rows to import",
    missing_file_content: "The request has no file content.",
    import_expired: "This import has expired. Choose the file again",
    source_name_length: "Source names are 1–60 characters",
    not_a_dataset: "Only dataset sources can be replaced",
    source_not_found: "That source doesn't exist",
    unknown_basis: (d: { value: string }) =>
      `Unrecognized basis value "${d.value}" (use 100g or serving)`,
    not_a_number: (d: { field: string }) => `${d.field} isn't a number`,
    missing_field: (d: { field: string }) => `Required field ${d.field} is missing`,
    missing_energy: "Required field kcal or kj is missing",
    not_positive: (d: { field: string; got: number }) =>
      `${d.field} must be greater than 0 (got ${d.got})`,
    kcal_kj_mismatch: (d: { kcal: number; kcal_from_kj: number }) =>
      `kcal (${d.kcal.toFixed(0)}) differs from the kj value converted to kcal (${d.kcal_from_kj.toFixed(0)}) by more than 2%; using kcal`,
    energy_mismatch: (d: { kcal: number; estimated: number }) =>
      `kcal (${d.kcal.toFixed(0)}) is far from the energy estimated from the macros (${d.estimated.toFixed(0)}); imported as is`,
    too_many_aliases: (d: { max: number }) => `More than ${d.max} aliases; the rest were ignored`,
    brand_on_food_row:
      "The brand column only applies to dish rows (basis=serving); ignored on this 100g row",
    food_name_empty: "The food name is empty",
    food_name_too_long: (d: { max: number }) => `The food name is longer than ${d.max} characters`,
    dish_name_empty: "The dish name is empty",
    dish_name_too_long: (d: { max: number }) => `The dish name is longer than ${d.max} characters`,
    brand_too_long: (d: { max: number }) => `The brand name is longer than ${d.max} characters`,
    duplicate_food: "Same food name as an earlier row in the file",
    duplicate_dish: "Same dish (brand and name) as an earlier row in the file",
    duplicate_food_alias: (d: { alias: string }) =>
      `Alias "${d.alias}" matches another food name or alias in the file; ignored`,
    duplicate_dish_alias: (d: { alias: string }) =>
      `Alias "${d.alias}" matches another dish name or alias of the same brand; ignored`,
    trace_values: (d: { rows: number }) => `${d.rows} rows use "Tr" (trace) values, read as 0`,
    unknown_columns: (d: { columns: string[] }) => `Ignored unknown columns: ${d.columns.join(", ")}`,
  },
  macroViolations: {
    negative: "All values must be numbers ≥ 0",
    fiber_exceeds_carbs: (d: { fiber_g: number; carbs_g: number }) =>
      `Fiber (${d.fiber_g}) can't exceed carbs (${d.carbs_g}): carbs are total carbohydrate, fiber included`,
    macros_exceed_100g: (d: { sum: number }) =>
      `Protein + carbs + fat can't exceed 100 per 100 g (got ${d.sum})`,
    kcal_per_100g_too_high: (d: { got: number }) => `kcal can't exceed 900 per 100 g (got ${d.got})`,
    kcal_per_serving_too_high: (d: { got: number }) =>
      `kcal can't exceed 5000 per serving (got ${d.got})`,
    serving_not_positive: "The serving weight must be greater than 0",
    macros_exceed_serving: (d: { sum: number; serving_g: number }) =>
      `Protein + carbs + fat (${d.sum}) can't exceed the serving weight (${d.serving_g} g)`,
    kcal_exceeds_serving: (d: { kcal: number; serving_g: number }) =>
      `kcal (${d.kcal}) can't exceed 9 × the serving weight (${d.serving_g} g)`,
  },
};
