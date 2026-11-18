# Agent Sessions over herdr — Implementation Plan (build step 1 of 4)

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A macOS Tauri app that shows all herdr agents grouped by Space, keeps a "Needs you" list with Dock badge and notifications, and lets the user answer an agent through a reply panel (with Approve/Deny) or a live terminal tab.

**Architecture:** The Rust core holds the herdr socket connection (NDJSON over a Unix socket), keeps an in-memory session model built from `session.snapshot` plus `events.subscribe`, derives attention state, and drives the Dock badge and notifications. The React UI only renders `UiState` pushed from Rust and calls typed Tauri commands. Features live in module folders behind a small module contract.

**Tech Stack:** Tauri 2, Rust (tokio, serde_json, thiserror), React + TypeScript + Vite (pnpm), Radix UI primitives, xterm.js, Vitest + Testing Library + vitest-axe, `tauri-plugin-notification`.

**Spec:** `docs/superpowers/specs/2026-11-18-personal-agents-ui-design.md`

## Global Constraints

- macOS only, Tauri 2.
- Talk to herdr only through its socket API: newline-delimited JSON, one request per line, `{"id","method","params"}` → `{"id","result"}` or `{"id","error":{"code","message"}}`. Default socket `~/.config/herdr/herdr.sock`; `HERDR_SOCKET_PATH` overrides it. Default herdr session only.
- The committed schema `docs/herdr/herdr-api.schema.json` (from `herdr api schema --json`) is the source of truth for method params and event shapes. Where this plan and the schema disagree, follow the schema and note it in the commit message.
- Ignore unknown fields in herdr payloads. Unknown agent status values map to `unknown`.
- Agent states: `working`, `blocked`, `done`, `idle`, `unknown`. In the UI, state is always shown as a dot **and** a text label ("Blocked", "Working", "Done", "Idle", "Unknown").
- herdr is the source of truth for Space order. Reordering in the app calls `workspace.move`; the app never closes or creates Spaces/panes on its own in this step.
- Closing the window hides it; Cmd+Q quits. Notifications and the Dock badge keep working while hidden.
- The app tracks "seen" itself: a `done` agent leaves "Needs you" once its panel is opened in the app. Opening a panel must not change focus in herdr's TUI.
- Accessibility: every action reachable by keyboard; visible focus ring; Radix primitives for tabs/menus; respect `prefers-color-scheme` and `prefers-reduced-motion`; vitest-axe passes on every rendered screen.
- No direct herdr calls from TypeScript; no `herdr` CLI subprocess per action from Rust (the CLI is only used in Task 1 to install/inspect).

## Review Focus

1. **herdr server not running or restarted mid-session.** The app shows "herdr isn't running" with the command to start it, reconnects on its own (backoff 0.5s → 5s) and re-snapshots, with no crash and no duplicate notifications. Tested in Task 3 and Task 4.
2. **An agent already blocked when the app starts or reconnects.** It appears in Needs you, but **no** notification fires. Notifications are only for transitions seen live. Tested in Task 4.
3. **Spaces reordered or closed in herdr's TUI.** The app's order follows the `workspace.moved` list, and a closed Space disappears along with its agents. Tested in Task 2.
4. **The agent's pane closes while its panel is open.** The panel shows "Session ended", and the reply box, Approve/Deny and terminal input are disabled. Tested in Task 6.
5. **Multi-line, unicode, or very long replies.** These are sent intact, and empty or whitespace-only replies are disabled. Tested in Task 6.

---

## File Structure

```
package.json, vite.config.ts, tsconfig.json, index.html
src/
  main.tsx                      app bootstrap
  App.tsx                       layout: Sidebar + main panel; registers modules
  styles.css                    tokens (light/dark), focus ring, reduced motion
  core/types.ts                 TS mirror of Rust UiState and friends
  core/ipc.ts                   typed wrappers over invoke()/listen()
  core/modules.ts               AppModule contract + registry
  core/StateDot.tsx             dot + text label for AgentStatus
  modules/agents/index.ts       agents AppModule
  modules/agents/Sidebar.tsx    Needs you + Spaces list (reorder)
  modules/agents/AgentPanel.tsx tabs: Conversation | Terminal (Changed files comes in step 2)
  modules/agents/Conversation.tsx
  modules/agents/TerminalView.tsx
  modules/agents/*.test.tsx
  test/setup.ts                 testing-library + vitest-axe setup, ipc mock helper
src-tauri/
  Cargo.toml, tauri.conf.json, capabilities/default.json
  src/main.rs                   calls lib::run()
  src/lib.rs                    Tauri builder, plugins, window-close-hides, spawns session supervisor
  src/state.rs                  AppState (model + attention + client handle), UiState builder
  src/commands.rs               #[tauri::command] fns
  src/herdr/mod.rs
  src/herdr/protocol.rs         Request/Incoming parsing
  src/herdr/client.rs           HerdrClient: call(), subscribe()
  src/herdr/model.rs            SessionModel, apply_event, diff
  src/herdr/supervisor.rs       connect → snapshot → subscribe → reconnect loop
  src/agents/attention.rs       needs-you, seen tracking, notify decision
  src/agents/prompt.rs          output cleaning, permission prompt detection
  tests/fake_herdr.rs           in-process fake herdr Unix-socket server (test helper)
  tests/client_test.rs, tests/supervisor_test.rs
  tests/fixtures/*.json, *.txt  captured from a real herdr
docs/herdr/herdr-api.schema.json
```

---

### Task 1: Toolchain, herdr, and app scaffold

**Files:**
- Create: the scaffold listed above (empty modules), `docs/herdr/herdr-api.schema.json`, `src/App.test.tsx`
- Modify: `README.md` (add "Develop" section: prerequisites, `pnpm tauri dev`, `pnpm test`, `cargo test`), `.gitignore` (`node_modules/`, `dist/`, `src-tauri/target/`)

**Interfaces:**
- Produces: `pnpm test` (vitest), `cargo test --manifest-path src-tauri/Cargo.toml`, and `pnpm tauri dev` all work. The app window is titled "Agents".

- [ ] **Step 1: Install prerequisites.** Rust via rustup (stable). Install herdr using https://herdr.chefgroep.nl/docs/install/ (prefer Homebrew; **ask the user before running a `curl | sh` installer**). Then run `herdr integration install claude`.
  Verify: `cargo --version`, `herdr --version`, and `herdr status` show a running server after `herdr` has been launched once in a terminal.
- [ ] **Step 2: Capture the schema.** `mkdir -p docs/herdr && herdr api schema --output docs/herdr/herdr-api.schema.json`. Write down the herdr version in the commit message.
- [ ] **Step 3: Scaffold** with `pnpm create tauri-app` (React + TypeScript, pnpm) into the repo root. Add `vitest`, `@testing-library/react`, `@testing-library/user-event`, `jsdom`, `vitest-axe`, `@radix-ui/react-tabs`, `@xterm/xterm`, and on the Rust side `tokio` (full), `serde`, `serde_json`, `thiserror`, `tauri-plugin-notification`.
- [ ] **Step 4: Write the failing test** `src/App.test.tsx`: render `<App />` with the ipc mock returning `{connection:"connecting", spaces:[], needsYou:[]}`, then assert `getByRole("navigation", {name: "Agents"})` exists and `axe` reports no violations.
- [ ] **Step 5: Run** `pnpm test`. Expected: FAIL (no nav).
- [ ] **Step 6: Implement** the minimal `App.tsx` with `<nav aria-label="Agents">` and an empty `<main>`, plus `test/setup.ts`.
- [ ] **Step 7: Run** `pnpm test` (PASS), `cargo test --manifest-path src-tauri/Cargo.toml` (PASS, 0 tests), and `pnpm tauri dev` (a window titled "Agents" opens).
- [ ] **Step 8: Commit** `chore: scaffold Tauri app and capture herdr API schema`.

### Task 2: herdr protocol and session model (pure Rust)

**Files:**
- Create: `src-tauri/src/herdr/{mod,protocol,model}.rs`, `src-tauri/tests/fixtures/{snapshot.json,events.ndjson}`
- Test: unit tests in `model.rs` and `protocol.rs`

**Interfaces:**
- Produces:
  - `protocol::Request { id: String, method: String, params: serde_json::Value }` with `to_line() -> String` (JSON plus `\n`).
  - `protocol::Incoming { Response { id: String, result: Value }, Error { id: String, code: String, message: String }, Event(HerdrEvent) }` and `protocol::parse_line(line: &str) -> Result<Incoming, ProtocolError>`.
  - `HerdrEvent { kind: String, data: Value }`. Take the envelope from the schema's event definitions.
  - `model::AgentStatus { Idle, Working, Blocked, Done, Unknown }`: serde lowercase, `#[serde(other)] Unknown`.
  - `model::Space { id: String, label: String, cwd: Option<String> }` (vector order is the herdr order).
  - `model::Agent { pane_id: String, workspace_id: String, tab_id: String, agent: Option<String>, title: Option<String>, status: AgentStatus, state_change_seq: u64 }`.
  - `model::SessionModel { spaces: Vec<Space>, agents: BTreeMap<String, Agent> }` with `from_snapshot(result: &Value) -> Result<Self, ProtocolError>`, `apply_event(&mut self, ev: &HerdrEvent) -> Vec<ModelChange>`, and `diff(old: &SessionModel, new: &SessionModel) -> Vec<ModelChange>`.
  - `model::ModelChange { SpacesChanged, AgentStatus { pane_id: String, old: Option<AgentStatus>, new: AgentStatus }, AgentRemoved { pane_id: String } }`.

- [ ] **Step 1: Capture fixtures from the real herdr.** Set up two Spaces, one with Claude Code running. Run `herdr api snapshot > src-tauri/tests/fixtures/snapshot.json`. For events, run `printf '{"id":"s","method":"events.subscribe","params":{"subscriptions":[...]}}\n' | nc -U ~/.config/herdr/herdr.sock > src-tauri/tests/fixtures/events.ndjson` with the workspace, pane and `pane.agent_status_changed` subscriptions from the schema. While it runs, make Claude ask for permission, reorder the Spaces in herdr, and close one pane. Take subscription params from the schema. Trim anything private (paths are fine).
- [ ] **Step 2: Write the failing tests:**
  - `parses_response_error_and_event_lines`: one line of each kind parses to the right `Incoming` variant, and an unknown extra field is ignored.
  - `snapshot_builds_spaces_in_order_and_agents`: from `snapshot.json`, the space count, first label and the Claude agent's `status` match the fixture.
  - `unknown_status_maps_to_unknown`: `"weird"` → `AgentStatus::Unknown`.
  - `status_event_yields_change_with_old_and_new`: applying the captured blocked event returns `AgentStatus{old: Some(Working), new: Blocked}` (or whatever the fixture holds).
  - `workspace_moved_reorders_spaces`: after the captured `workspace.moved`, `spaces` matches the event's ordered `workspaces` list.
  - `workspace_closed_removes_space_and_its_agents` and `pane_closed_removes_agent` (each emits `AgentRemoved`).
  - `diff_reports_status_changes_and_removals`: two hand-built models give the expected changes, and identical models give an empty vec.
- [ ] **Step 3: Run** `cargo test --manifest-path src-tauri/Cargo.toml herdr::`. Expected: FAIL (types missing).
- [ ] **Step 4: Implement** `protocol.rs` and `model.rs`. Field names and the event envelope come from the schema and fixtures. Events the model doesn't use return an empty vec.
- [ ] **Step 5: Run the tests.** Expected: PASS.
- [ ] **Step 6: Commit** `feat(herdr): protocol parsing and session model`.

### Task 3: herdr client and supervisor (socket, reconnect)

**Files:**
- Create: `src-tauri/src/herdr/{client,supervisor}.rs`, `src-tauri/tests/{fake_herdr.rs,client_test.rs,supervisor_test.rs}`

**Interfaces:**
- Consumes: `protocol::*` and `model::*` from Task 2.
- Produces:
  - `client::default_socket_path() -> PathBuf`: `HERDR_SOCKET_PATH` if set, else `~/.config/herdr/herdr.sock`.
  - `client::HerdrError { NotRunning, Disconnected, Api { code: String, message: String }, Protocol(String) }` (thiserror).
  - `client::HerdrClient`: `Clone`, with `connect(path: &Path) -> Result<Self, HerdrError>` and `async call(&self, method: &str, params: Value) -> Result<Value, HerdrError>`. Requests are correlated by id, so concurrent calls on one connection are fine.
  - `client::subscribe(path: &Path, subscriptions: Value) -> Result<mpsc::Receiver<HerdrEvent>, HerdrError>`: a dedicated connection; the receiver closes when the socket closes.
  - `supervisor::SessionUpdate { Connecting, NotRunning, Connected { client: HerdrClient, model: SessionModel, changes: Vec<ModelChange>, live: bool } }`. `live` is false for the first snapshot after each (re)connect.
  - `supervisor::run(path: PathBuf, tx: mpsc::Sender<SessionUpdate>)`: loop forever. Connect, call `session.snapshot`, subscribe, then send `Connected` after the snapshot (with `changes` = `diff(previous model, new)` and `live: false`) and after each event (with `live: true`). On error, send `NotRunning` or `Connecting` and retry with backoff starting at 500 ms, doubling, capped at 5 s, and reset after a successful connect.
- `fake_herdr.rs` test helper: `FakeHerdr::start() -> (FakeHerdr, PathBuf)` binds a `UnixListener` in a temp dir; `.on(method, result_json)` scripts responses, `.push_event(json)` writes to subscribed connections, and `.stop()` / `.restart()` close and rebind.

- [ ] **Step 1: Write the failing tests:**
  - `call_returns_result_and_maps_error`: a `ping` gives `{"type":"pong"}`, and a scripted error gives `HerdrError::Api{code:"not_found",..}`.
  - `concurrent_calls_are_correlated`: two calls answered in reverse order each get their own result.
  - `connect_to_missing_socket_is_not_running`.
  - `subscribe_receives_pushed_events`.
  - `supervisor_reconnects_after_restart`: with the fake serving a snapshot, the first update is `Connected{live:false}`. After `stop()` comes `NotRunning`/`Connecting`. After `restart()` with an agent changed to blocked, the update is `Connected{live:false}` and its `changes` include that status change. Use a 2 s test timeout.
- [ ] **Step 2: Run** `cargo test --manifest-path src-tauri/Cargo.toml --test client_test --test supervisor_test`. Expected: FAIL.
- [ ] **Step 3: Implement** `client.rs` (one reader task that routes responses to `oneshot` senders kept in a `HashMap<String, oneshot::Sender<…>>`, with ids from an `AtomicU64`) and `supervisor.rs`.
- [ ] **Step 4: Run the tests.** Expected: PASS.
- [ ] **Step 5: Smoke against the real herdr:** add an ignored test `real_herdr_ping` (`#[ignore]`) and run it with `--ignored`. Expected: pong.
- [ ] **Step 6: Commit** `feat(herdr): socket client with reconnecting supervisor`.

### Task 4: Attention, app state, Dock badge, notifications, window lifecycle

**Files:**
- Create: `src-tauri/src/agents/{mod,attention}.rs`, `src-tauri/src/state.rs`
- Modify: `src-tauri/src/lib.rs`, `src-tauri/capabilities/default.json` (notification permission)

**Interfaces:**
- Consumes: `SessionUpdate`, `SessionModel`, `ModelChange`, `supervisor::run`.
- Produces:
  - `attention::Attention { seen: HashSet<(String, u64)> }` with `mark_seen(&mut self, pane_id: &str, state_change_seq: u64)`, `needs_you(&self, model: &SessionModel) -> Vec<NeedsYouItem>`, and `should_notify(change: &ModelChange, live: bool) -> bool`.
  - `NeedsYouItem { pane_id, workspace_id, space_label: String, title: Option<String>, status: AgentStatus }`. The list holds blocked agents, then done agents that aren't seen, each group sorted by `state_change_seq` descending.
  - `state::UiState` (serde camelCase): `{ connection: "connecting" | "connected" | "notRunning", spaces: Vec<SpaceView>, needsYou: Vec<NeedsYouItem> }`, where `SpaceView { id, label, agents: Vec<Agent> }` follows herdr order.
  - `state::AppState` (in a `tokio::sync::Mutex`, managed by Tauri) holds `Option<HerdrClient>`, `SessionModel`, `Attention`, and the connection status, with `to_ui(&self) -> UiState`.
  - Tauri event `"state-changed"` with payload `UiState`, emitted after every `SessionUpdate`.
  - The Dock badge equals the count of blocked agents in `needsYou` (no badge at 0) and is set via the main window's `set_badge_count`.

- [ ] **Step 1: Write the failing tests** (unit tests in `attention.rs`):
  - `needs_you_orders_blocked_then_unseen_done`.
  - `seen_done_is_hidden_until_new_state_change`: after `mark_seen(p, 7)`, an agent with seq 7 is hidden and the same pane at seq 9 shows again.
  - `notify_only_on_live_transition_into_blocked`: `(Working→Blocked, live)` gives true, `(Working→Blocked, not live)` gives false, `(Blocked→Blocked, live)` gives false, and `(None→Blocked, live)` (a new pane that is already blocked) gives true.
  - `idle_and_working_never_in_needs_you`.
- [ ] **Step 2: Run** `cargo test --manifest-path src-tauri/Cargo.toml attention`. Expected: FAIL.
- [ ] **Step 3: Implement** `attention.rs` and `state.rs`. In `lib.rs`: register the notification plugin, spawn `supervisor::run(default_socket_path(), tx)`, and consume updates in a task that updates `AppState`. For each change where `should_notify` is true, post a notification titled `"<agent title or 'Agent'> needs you"` with the Space label as the body. Then set the badge and emit `"state-changed"`. On `WindowEvent::CloseRequested`, call `api.prevent_close()` and hide the window. On reopen (`RunEvent::Reopen`), show and focus it. The spec requires that clicking a notification opens that agent's panel. Show the window and emit `"select-agent"` with the `pane_id`, and have `App.tsx` select it (wired in Task 5). If `tauri-plugin-notification` can't deliver click events on macOS desktop, show the window on activation and report the gap to the user instead of working around it.
- [ ] **Step 4: Run the tests.** Expected: PASS.
- [ ] **Step 5: Manual check** with `pnpm tauri dev` and herdr running. Hide the window with Cmd+W and make Claude ask for permission. Expected: one notification, and the Dock badge shows 1. Quit herdr (`herdr server stop`): no crash. Relaunch herdr: no new notification for the already-blocked agent.
- [ ] **Step 6: Commit** `feat(agents): attention tracking, Dock badge, notifications, hide-on-close`.

### Task 5: Commands, module contract, and sidebar UI

**Files:**
- Create: `src-tauri/src/commands.rs`, `src/core/{types,ipc,modules}.ts`, `src/core/StateDot.tsx`, `src/modules/agents/{index.ts,Sidebar.tsx,Sidebar.test.tsx}`, `src/styles.css`
- Modify: `src/App.tsx`, `src-tauri/src/lib.rs` (register commands)

**Interfaces:**
- Consumes: `AppState`, `UiState`, `"state-changed"`.
- Produces (Rust commands → TS wrappers in `ipc.ts`, same names in camelCase):
  - `get_state() -> UiState` → `getState(): Promise<UiState>`
  - `mark_seen(pane_id: String)` → `markSeen(paneId)`: uses the agent's current `state_change_seq` and re-emits state.
  - `move_space(workspace_id: String, insert_index: usize)` → `moveSpace(id, index)`: calls `workspace.move` (params per schema). The UI waits for the herdr event rather than reordering optimistically.
  - `onStateChanged(cb: (s: UiState) => void): Promise<UnlistenFn>`
  - `types.ts` mirrors `UiState`, `SpaceView`, `Agent`, `NeedsYouItem`, `AgentStatus`.
  - `modules.ts`: `interface AppModule { id: string; title: string; Sidebar?: React.FC<{selection: Selection; onSelect(s: Selection): void}>; Main?: React.FC<{selection: Selection}> }` with `type Selection = { module: string; id: string } | null` and `const modules: AppModule[]`. Storage and event hooks are added to this contract in build step 3, when the first module needs them.
  - `useUiState(): UiState` hook in `ipc.ts` (initial `getState()` plus a subscription).
  - `onSelectAgent(cb: (paneId: string) => void)` in `ipc.ts`. It listens for the `"select-agent"` event from Task 4, and `App.tsx` uses it to set the selection.

- [ ] **Step 1: Write the failing tests** `Sidebar.test.tsx` (mock `ipc`):
  - `shows needs-you section with count and items`: a heading "Needs you (2)" and items containing "Blocked" and "Done" as text.
  - `lists spaces in given order with agent state labels`.
  - `selecting an item calls onSelect and markSeen`.
  - `alt+arrow moves focused space`: Alt+↓ on the first Space calls `moveSpace(id, 1)`, and Alt+↑ on the first Space does nothing.
  - `drag and drop reorders` (pointer events, calls `moveSpace`).
  - `shows not-running banner`: with `connection:"notRunning"` the sidebar shows "herdr isn't running" and the text `herdr` (the command to start it).
  - `has no axe violations` (in both connected and not-running states).
- [ ] **Step 2: Run** `pnpm test`. Expected: FAIL.
- [ ] **Step 3: Implement** the commands, `ipc.ts`, `modules.ts`, `StateDot` (an `aria-hidden` dot plus the visible label), and `Sidebar` (a `<section aria-labelledby>` for Needs you; Spaces as a list of `role="listitem"` entries with focusable buttons; native HTML drag-and-drop plus Alt+↑/↓). In `styles.css`, use system font, light and dark tokens, a 2 px focus ring, and turn off transitions under `prefers-reduced-motion`.
- [ ] **Step 4: Run** `pnpm test` and `cargo test`. Expected: PASS.
- [ ] **Step 5: Commit** `feat(agents): sidebar with needs-you and ordered spaces`.

### Task 6: Agent panel — conversation, reply, Approve/Deny

**Files:**
- Create: `src-tauri/src/agents/prompt.rs`, `src-tauri/tests/fixtures/{claude_permission.txt,claude_idle.txt}`, `src/modules/agents/{AgentPanel.tsx,Conversation.tsx,AgentPanel.test.tsx}`
- Modify: `src-tauri/src/commands.rs`, `src/core/{ipc,types}.ts`

**Interfaces:**
- Consumes: `HerdrClient::call`, `AppState`, `Selection`.
- Produces:
  - `prompt::clean_output(raw: &str) -> String` strips ANSI escapes and control characters (keeping `\n`), drops trailing blank lines, and collapses runs of more than 2 blank lines.
  - `prompt::detect_permission(screen: &str) -> Option<PermissionPrompt { question: String }>`.
  - Commands:
    - `read_conversation(pane_id) -> Conversation { text: String, permission: Option<PermissionPrompt>, ended: bool }`: `pane.read` with source `recent-unwrapped`, last 200 lines, cleaned. Permission detection runs on source `detection`, and only while the status is `blocked`. If the pane doesn't exist, return `ended: true`.
    - `send_reply(pane_id, text)`: uses `agent.prompt`. Rejects empty or whitespace-only text.
    - `approve(pane_id)` / `deny(pane_id)`: use `pane.send_keys` with the key sequences defined as constants `APPROVE_KEYS` and `DENY_KEYS` in `prompt.rs`.
  - The UI `AgentPanel` has Radix Tabs "Conversation" and "Terminal", and a header with the Space label, title and `StateDot`. It calls `markSeen` on mount, and re-reads the conversation on every `state-changed` that touches this pane, at most every 500 ms.

- [ ] **Step 1: Capture fixtures.** Save `herdr pane read <pane> --source detection` while Claude shows a permission prompt (`claude_permission.txt`) and while it is idle (`claude_idle.txt`). Try which keys approve and deny on that prompt using `herdr pane send-keys`, then record them as `APPROVE_KEYS`/`DENY_KEYS` (expected to be `["1"]` and `["esc"]`; the fixture run decides).
- [ ] **Step 2: Write the failing tests:**
  - Rust: `clean_output_strips_ansi_and_trailing_blanks`, `detects_permission_in_fixture` (the question text is non-empty and contains the tool/command shown), `no_permission_when_idle`.
  - TS (`AgentPanel.test.tsx`):
    - `shows conversation text`.
    - `shows Approve and Deny only when permission present`, and they call `approve`/`deny`.
    - `reply sends exact multi-line unicode text`: the input `"line1\nline2 — ✓ 日本"` reaches `sendReply` unchanged. Enter sends, Shift+Enter adds a newline.
    - `reply disabled when empty or whitespace`.
    - `ended session disables inputs and shows Session ended`.
    - `marks seen on open`.
    - `no axe violations`.
- [ ] **Step 3: Run** `cargo test` and `pnpm test`. Expected: FAIL.
- [ ] **Step 4: Implement** `prompt.rs`, the commands, and the panel.
- [ ] **Step 5: Run the tests.** Expected: PASS. Then **manual check:** answer a real Claude permission prompt with Approve, and send a reply to a waiting agent.
- [ ] **Step 6: Commit** `feat(agents): conversation panel with reply and approve/deny`.

### Task 7: Terminal tab

**Files:**
- Create: `src/modules/agents/{TerminalView.tsx,TerminalView.test.tsx}`
- Modify: `src-tauri/src/commands.rs`, `src/core/ipc.ts`, `src/modules/agents/AgentPanel.tsx`

**Interfaces:**
- Produces:
  - `read_screen(pane_id) -> Screen { text: String, ended: bool }`: `pane.read` with source `visible`. Check in the schema whether it can return ANSI-styled output; if it can, request that, otherwise return plain text.
  - `send_input(pane_id, data: String)`: `pane.send_input` (params per schema). Printable text is sent as text, and special keys as herdr key-combo strings.
  - `keyToHerdr(e: {key, ctrlKey, altKey, shiftKey, metaKey}): string | null` in `TerminalView.tsx`. It maps Enter→`enter`, Escape→`esc`, Tab→`tab`, Shift+Tab→`shift+tab`, arrows→`up`/`down`/`left`/`right`, Backspace→`backspace`, Ctrl+letter→`ctrl+<letter>`, `-`→`minus`, `+`→`plus`, and returns null for Meta combos (those stay app shortcuts).
  - `TerminalView` renders into xterm.js. It refreshes with `read_screen` every 250 ms while the tab is visible and the window is focused, pauses otherwise, and redraws only when the text changed.
- Note: herdr has no raw PTY stream for outside clients, so this terminal is a polled view of the screen with input forwarded. Rendering is not byte-exact; that is accepted as the fallback in the spec.

- [ ] **Step 1: Write the failing tests:**
  - `keyToHerdr maps special keys` (table test over the mapping above).
  - `polls only while visible` (fake timers: no `readScreen` calls when hidden).
  - `ended disables input`.
- [ ] **Step 2: Run** `pnpm test`. Expected: FAIL.
- [ ] **Step 3: Implement** the commands and `TerminalView`. Clear and rewrite xterm only when the text differs.
- [ ] **Step 4: Run** `pnpm test`. Expected: PASS. Manual check: type a prompt into Claude through the Terminal tab and press Enter; the screen updates within about half a second.
- [ ] **Step 5: Commit** `feat(agents): terminal tab with polled screen and key forwarding`.

### Task 8: End-to-end verification

- [ ] **Step 1:** Run `cargo test --manifest-path src-tauri/Cargo.toml` and `pnpm test`. Expected: all PASS.
- [ ] **Step 2:** Run `pnpm tauri build`. The `.app` launches from `src-tauri/target/release/bundle/macos/`.
- [ ] **Step 3: Manual walkthrough** on the built app, once for each Review Focus item 1–5, plus:
  - full keyboard-only use (Tab, arrows, Enter, Alt+↑/↓)
  - VoiceOver reads state labels
  - dark mode
- [ ] **Step 4:** Add a short "Using it" section to `README.md`. Commit `docs: usage notes for agent sessions`.

---

## Later build steps (separate plans, written when step 1 ships)

2. **Changed files.** A Rust `git` module that diffs the agent's working folder against HEAD (or, from step 4, against the recorded base commit). Adds a "Changed files" tab with a file list and a Monaco diff editor, refreshed on agent state changes.
3. **Projects, backlog, todos.** A `Store` trait with SQLite (rusqlite plus migrations). Projects are matched to Spaces by folder path, with an "unlinked Space → adopt" flow and a personal list. Extends `AppModule` with storage migrations and event hooks.
4. **Todo → agent.** `worktree.create` per todo, an agent started with the todo as its prompt, and an `AgentLink` storing worktree, branch and base commit. Todo status follows the spec's rules, and Merge/Discard actions are added.
