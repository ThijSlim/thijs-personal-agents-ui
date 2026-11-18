# Personal Agents UI — Design

Date: 2026-11-18
Status: Draft, awaiting approval

## Intent

A personal macOS desktop app (Tauri) that Thijs runs his working day from:
his projects, his personal backlog and todo list, and the coding agents doing
work on them. Today agent sessions run in herdr, a terminal multiplexer whose
UI *is* a terminal; that is the main annoyance. This app puts a proper native
UI on top of herdr and joins it with personal planning, so a todo can be
handed to an agent on the spot and its progress, questions, and changed files
are visible without living in a terminal.

It is a personal productivity tool for one user. Success: it becomes the
default place he starts and steers his day's work.

## Goals

1. **Agent sessions, ordered by herdr Spaces.** See every agent across all
   projects with its state; projects map 1:1 to herdr Spaces, and ordering
   projects in the app orders the Spaces.
2. **"Needs you" visibility.** Agents that are `blocked` (need input) or
   `done` (finished, unseen) are always visible: a pinned list in the app, a
   Dock badge count, and a macOS notification when an agent becomes blocked.
3. **Respond to agents without a terminal by default.** A reply panel shows
   the agent's recent output as readable text, a reply box, and
   Approve/Deny buttons when the agent is asking for permission. A live
   embedded terminal is one click away as a fallback.
4. **Changed files per agent session.** A list of files changed in the
   session's working folder with a side-by-side diff (Monaco diff editor).
5. **Projects, backlog, todos.** Each project has a backlog/todo list; there
   is also one personal list not tied to any project.
6. **Todo → agent.** Assign a todo to an agent: the app starts an agent in
   the project's Space with the todo text as its first prompt, links the
   agent to the todo, and shows the agent's live state on the todo.
7. **Pluggable features.** New features are added as built-in modules with a
   clear contract (see Architecture), so the app grows without becoming
   tangled.
8. **Clean and accessible UI.** Minimal visual design; full keyboard
   operation; screen-reader labels; state never conveyed by colour alone;
   respects macOS light/dark mode and reduced motion.

## Non-goals

- Multi-user, team, or sharing features.
- Syncing todos with GitHub issues, Jira, Apple Reminders, etc.
- A hosted/remote database (planned later; see Constraints).
- Replacing herdr or reimplementing terminal multiplexing. herdr's own TUI
  keeps working alongside the app on the same sessions.
- A general code editor. Monaco is used for viewing diffs, not editing.
- Runtime-loaded third-party plugins or a stable public plugin API.
- Windows/Linux support.

## Anti-goals (would make it a failure even if it works)

- Ending up as "a terminal in a nicer frame": the default path for an agent
  must be the reply panel, not the terminal.
- A blocked agent going unnoticed because the app is in the background.
- Corrupting or fighting herdr state (e.g. the app's ordering drifting from
  herdr's Space order, or the app killing sessions the user started in
  herdr's TUI).
- Losing todos.

## Constraints

- **Platform:** macOS only, Tauri 2.
- **herdr is the runtime.** Not yet installed on this machine; install via
  Homebrew or the official installer (https://herdr.chefgroep.nl/docs/install/).
  The app talks to the running herdr server over its local socket API
  (https://herdr.chefgroep.nl/docs/socket-api/). Relevant facts from the docs:
  - Concepts: session → workspace (shown as "space" in recent herdr UI) →
    tab → pane; agents are detected inside panes with states `working`,
    `blocked`, `done`, `idle`, `unknown`.
  - Methods include `session.snapshot` (bootstrap), `events.subscribe`,
    `workspace.create|list|rename|move|close|report_metadata`,
    `agent.list|get|read|prompt|send_keys|start|wait|focus`,
    `pane.read|send_text|send_keys|wait_for_output`, `worktree.*`,
    `plugin.*`.
  - The exact schema is bundled with the binary: `herdr api schema --json`.
    **The builder must generate/verify the client against this schema**
    rather than trusting this summary.
  - If the herdr server isn't running, the app should start it (or tell the
    user how) rather than failing silently.
- **Storage:** local SQLite file in the app's data directory for now. A
  hosted database is expected later, so all persistence goes through one
  storage interface; no screen or module touches SQL directly.
- **Agents:** Claude Code is the primary agent. Install herdr's Claude
  integration (`herdr integration install claude`) for authoritative state.
  Other herdr-supported agents should work where herdr supports them, but
  aren't a target for special UX.

## Approaches considered

**Overall architecture**

1. **Rust core owns connections; UI only displays and sends commands.**
   *(Chosen.)* The Tauri Rust side holds one persistent herdr socket
   connection and event subscription, owns SQLite, runs git for changed
   files, and drives Dock badge/notifications. The React UI subscribes to
   state from Rust and invokes commands. Picked because it is the most
   maintainable: herdr protocol handling and data access live in one tested
   place, live updates work while the window is hidden, and a herdr update
   is fixed in one module. Cost: more Rust up front.
2. **Thin Tauri shell; logic in the TypeScript UI**, shelling out to the
   `herdr` CLI per action and using a SQL plugin from the frontend. Faster
   to a first screen, but a process per action, awkward live updates, and
   logic smeared across screens. Rejected for maintainability.

**Feature pluggability**

1. **Built-in modules** with a typed contract, compiled into the app.
   *(Chosen — user asked for "what is best for maintainability".)* One
   codebase, one compiler, no external API to keep stable.
2. **Runtime-loaded plugins** with a published plugin API. Rejected: much
   more work and only pays off with third-party authors.
3. **herdr plugins** remain available for features that should also work
   inside herdr's own TUI; not the app's extension mechanism.

**Responding to agents** (user chose C)

- A. Embedded terminal only. B. Reply panel only. **C. Reply panel by
  default, embedded terminal one click away.**

## Architecture

### Rust core (src-tauri)

- **herdr client:** connects to the herdr socket, loads `session.snapshot`,
  subscribes to events, keeps an in-memory model of spaces/tabs/panes/agents,
  and reconnects + re-snapshots on disconnect. Exposes typed commands
  (create space, move space, start agent with prompt, send reply, approve,
  deny, read output) to the UI and emits model-changed events.
- **Storage:** a `Store` trait with a SQLite implementation (migrations
  included). Holds projects, todos, the personal list, and todo↔agent links.
- **Changed files:** for a session's working folder, run git to list changed
  files and fetch original/modified contents for the diff view. Refresh on
  agent state changes and on demand.
- **Attention:** derives the "Needs you" set from agent states; sets the Dock
  badge; posts a macOS notification on transition into `blocked`.
  Clicking a notification opens that agent's panel.

### UI (React + TypeScript)

- Accessible primitives: Radix UI, styled minimally. Monaco for diffs,
  xterm.js for the terminal tab.
- Layout sketch:

```
┌──────────────┬──────────────────────────────────────────┐
│ Needs you (2)│  api-refactor · "Fix auth tokens" ● Blocked
│ ● Blocked …  │  [Conversation] [Changed files (3)] [Terminal]
│ ● Done    …  │                                          │
│──────────────│  Agent: I want to run `npm test`. Allow? │
│ Projects     │   [Approve] [Deny]                       │
│  api-refactor│  ┌────────────────────────────────────┐  │
│  website     │  │ Reply…                             │  │
│ Personal list│  └────────────────────────────────────┘  │
└──────────────┴──────────────────────────────────────────┘
```

- Sidebar: "Needs you" pinned on top; projects in Space order (drag or
  keyboard to reorder); personal list.
- Project view: backlog/todos with "Assign to agent" on each todo; the
  project's agents with state labels.
- Agent panel: Conversation (reply panel), Changed files (list + Monaco
  diff), Terminal (live xterm.js).

### Module contract

Each feature module (todos, agents, changed-files, and future ones) lives in
its own folder on both sides and declares: its UI routes/panels, its
navigation entries, its storage tables/migrations, and the core events it
listens to (e.g. `todo.assigned`, `agent.state_changed`). Modules talk to
each other only through these events and the core's typed commands, not by
importing each other's internals.

### Data model (initial)

- **Project:** id, name, folder path, herdr space id, order.
- **Todo:** id, project id (nullable → personal list), title, notes,
  status (backlog / todo / in progress / done), order, created/updated.
- **AgentLink:** todo id, herdr pane/agent id, worktree path, branch, base commit, started at, ended at (nullable).

### Project ↔ Space mapping

- Creating a project creates (or adopts) a herdr Space for its folder.
- Reordering projects calls `workspace.move`; herdr Space order changes made
  in herdr's TUI are reflected back in the app (herdr is the source of truth
  for order).
- Spaces in herdr with no matching project are shown as "unlinked" and can be
  adopted as a project.

### Worktree per assigned todo (user decision)

- "Assign to agent" creates a git worktree on a new branch for that todo
  (via herdr's `worktree.*` methods where they fit), starts the agent in it,
  and records worktree path, branch, and base commit on the AgentLink.
- "Changed files" for that agent diffs the worktree (committed and
  uncommitted) against the recorded base commit, so it is correct even if
  the agent commits and even with parallel agents on one project.
- The agent panel offers "Merge into <base branch>" and "Discard" once the
  work is reviewed; both remove the worktree. Merge conflicts are surfaced,
  not auto-resolved.
- Agents started directly in a project folder (not from a todo) show
  uncommitted changes against HEAD.

### Decisions settled during review (designer's calls, user may override)

- **Identity and reconciliation.** A project's identity is its folder path;
  the herdr space id is a cached pointer. On startup/reconnect the app
  re-matches projects to Spaces by folder. If a project's Space is gone, the
  project is shown as "not running" with a one-click "Open space" that
  recreates it; the app never recreates Spaces on its own. AgentLinks to
  agents that no longer exist are kept as history (shown as "ended").
- **Todo status vs. agent state.** Assigning a todo moves it to
  "in progress". When its agent reaches `done`, the todo shows "Agent done —
  review" but its status only changes when the user marks it done. A todo
  may have several AgentLinks (e.g. a retry); the most recent is shown.
- **Seen / unseen.** The app tracks "seen" itself: a `done` agent leaves
  "Needs you" once the user opens its panel in the app. The app does not
  change focus in herdr's TUI to achieve this.
- **herdr sessions.** The app attaches to herdr's default session only.
- **Background behaviour.** Closing the window keeps the app running (Dock
  icon and badge stay, notifications still fire); Cmd+Q quits. A "Launch at
  login" setting is offered, off by default.
- **Personal-list todos.** "Assign to agent" asks which project to run it
  in; the todo stays on the personal list.

## Build order

1. Agent sessions over herdr (Rust client, Spaces, Needs you, reply panel,
   terminal fallback).
2. Changed files (git + Monaco diff).
3. Projects, backlog, todos (SQLite store).
4. Todo → agent assignment.

## Left to the builder

- Exact herdr method/params, verified against `herdr api schema --json`.
- How to detect a permission prompt for Approve/Deny (e.g. herdr agent state
  plus Claude integration data, or screen parsing) and what keys
  Approve/Deny send.
- How the terminal tab gets a live stream (e.g. herdr's pane read/output
  events, or embedding a herdr client attach in a PTY).
- Rendering "recent output as readable text" (stripping ANSI, trimming
  chrome).
- Styling details, icon, app name.
- Test strategy and tooling.
