# LifeAgent — Architecture Overview

This document describes the whole project at a high level. The detailed, implementable
specification for the current phase is `PHASE1_SPEC.md`.

## 1. Vision

LifeAgent is a personal AI agent for Windows that helps with daily life. It is not a diet app
with AI added on top; it is an **agent platform** whose capabilities come from plugins. The
first plugins cover health: diet, exercise, alcohol and smoking. Each evening the agent reviews
the day and produces a concrete plan for tomorrow that moves the user toward their goals.
Later plugins (finance, schedule, learning, habits, ...) plug into the same core without
changing it.

The user interacts mainly by talking to it in natural language ("午饭吃了一碗牛肉面和一个卤蛋",
"跑了 5 公里", "晚上喝了两罐啤酒"), and sees progress and plans on a dashboard.

## 2. Design principles

1. **Plugin-first.** Every domain feature is a plugin. The core knows nothing about food,
   workouts or cigarettes.
2. **Numbers by code, words by the LLM.** Calories, macros, targets, training load and limits
   are computed deterministically. The LLM parses free text, chooses foods and exercises,
   and writes explanations. Every LLM output that affects numbers passes validation.
3. **Replaceable AI core.** All model access goes through one interface. Models are assigned
   per *role* (parser, planner, conversation) and can be swapped in settings.
4. **Local-first data.** All user data lives in a local SQLite file. Only minimized summaries
   and the current conversation are sent to cloud models.
5. **Human in the loop.** Plans are proposals. The user can revise them by talking to the agent.
   Differences between plan and reality feed back into the next plan.
6. **UI-agnostic core.** The core is a library. The desktop app, a developer CLI and a future
   mobile or web client are thin shells around it.
7. **Safe defaults.** Hard floors and ceilings (minimum calories, maximum weekly loss, maximum
   training load increase) are enforced in code regardless of what the LLM proposes. The app
   gives lifestyle guidance, not medical advice.

## 3. System structure

```
┌──────────────────────────────────────────────────────────────┐
│ Shells: Desktop (Tauri + thin React UI) · CLI · future mobile │
└──────────────────────────────┬───────────────────────────────┘
                               │ Rust API
┌──────────────────────────────▼───────────────────────────────┐
│ Core                                                          │
│  Orchestrator (tool-calling loop)   Context builder / memory  │
│  Tool registry   Event bus   Plan service   Scheduler         │
│  Storage (SQLite)   Settings   Secrets   Clock                │
└───────────────┬──────────────────────────────┬───────────────┘
                │ Plugin trait                 │ ChatModel trait
┌───────────────▼──────────────┐   ┌───────────▼───────────────┐
│ Plugins                      │   │ LLM layer                  │
│  diet · exercise · alcohol · │   │  role router · providers · │
│  smoking · (future) ...      │   │  retries · usage tracking  │
│  (later) external via MCP    │   │                            │
└──────────────────────────────┘   └────────────────────────────┘
```

## 4. Components

### Core
- **Orchestrator** — receives a user message, builds context, calls the conversation model
  with all registered tools, executes tool calls, loops until the model answers.
- **Tool registry** — collects tools from core and plugins; routes calls by name.
- **Event bus** — plugins publish domain events (e.g. `diet.meal_logged`) and subscribe to
  others' events. Plugins never call each other directly.
- **Context builder / memory** — assembles the system prompt: time, profile, goals, today's
  status (contributed by each plugin), current plan, and long-term memory notes.
- **Plan service** — gathers plan inputs from all plugins, makes one planner LLM call so the
  plan is coherent across domains, hands each section back to its plugin for resolution and
  validation, retries on violations, stores the result.
- **Scheduler** — runs the nightly plan generation and reminders; catches up after downtime.
- **Storage** — SQLite: generic event log, profile, goals, plans, conversation, memory,
  settings, LLM usage; plugins add their own namespaced tables.

### LLM layer
- One `ChatModel` trait; Phase 1 ships two implementations — OpenAI-compatible Chat
  Completions and Anthropic Messages. Further providers (Gemini, etc.) plug in the same way.
- A router maps roles to (provider, model): **parser** (cheap, frequent), **planner**
  (strongest, nightly), **conversation** (mid-tier). Later: per-role fallback models, monthly
  budget cap.
- Every call is logged with token usage for cost visibility.

### Plugins
A plugin declares and implements: its event types and tables, tools for the LLM, a status
snippet for the agent's context, a dashboard card payload, optional plan input and plan
section finalization, and optional event handlers. Phase 1–3 plugins are Rust crates compiled
into the app. Phase 4 adds external plugins through MCP (Model Context Protocol), so existing
MCP servers and plugins written in any language can be attached without recompiling.

### Shells
- **Desktop**: Tauri 2. Tray icon, close-to-tray so the scheduler keeps running, Windows
  notifications, optional autostart, quick-entry window.
- **CLI**: developer harness for chatting, triggering plans and inspecting data.

## 5. Data model (conceptual)

| Entity | Purpose |
|---|---|
| Event | Every user record: time, logical date, type (`plugin.name`), JSON payload |
| Profile | Body data, activity level, allergies, dislikes, injuries |
| Goal | Domain goals with JSON parameters (weight target, weekly alcohol units, cigarettes/day taper) |
| Plan | One per date, versioned; sections keyed by plugin id; warnings |
| Memory note | Durable preferences and facts the agent should remember |
| Conversation | Chat messages including tool calls |
| LLM usage | Tokens per call, role, model |

## 6. Daily loop

1. During the day the user logs things by chat or quick buttons; parser + plugin tools turn
   them into events; the dashboard updates live.
2. At the configured time (default 21:00) the scheduler triggers plan generation for tomorrow.
3. Each plugin computes deterministic targets and constraints; the planner writes one coherent
   plan; plugins resolve and validate their sections; violations trigger one guided retry.
4. The user gets a notification, reviews the plan, and can revise it in conversation.
5. The next plan includes plan-vs-actual adherence from recent days, so plans adapt.

## 7. Roadmap

| Phase | Scope |
|---|---|
| **1 — Core loop** | Core library, OpenAI-compatible and Anthropic providers with role routing, diet plugin, chat logging, weight logging, nightly diet plan with validation and revision, Today / Chat / Settings pages, tray + notifications, CLI |
| **2 — Health suite** | Exercise, alcohol and smoking plugins; cross-domain plan (training + diet + reduction strategies); pattern detection (e.g. drinking concentrated on Fridays); quick-log buttons |
| **3 — Insight & control** | Trends page (weekly/monthly charts), further providers (Gemini, ...), per-role fallback models, cost dashboard and monthly budget cap, food database import (e.g. USDA FoodData Central), meal photo logging via multimodal models |
| **4 — Open platform** | External plugins via MCP, plugin enable/disable and settings UI, first non-health plugins, optional sync/mobile access |

## 8. Non-goals and constraints

- Single user, single machine, Windows first (the core stays cross-platform).
- No medical diagnosis or treatment advice; the app surfaces a short disclaimer and suggests
  consulting a professional when the profile lists medical conditions.
- No local model inference in the near term (cloud APIs only), but nothing in the design
  prevents adding a local provider later.
