# Agent sessions: manual verification checklist

Run against the bundled app (`src-tauri/target/release/bundle/macos/*.app`) with herdr running.
Tick each item; note anything unexpected.

## Review Focus

1. herdr not running or restarted mid-session
   - [ ] Start the app with herdr stopped: it shows "herdr isn't running" with the command to start it.
   - [ ] Start herdr: the app reconnects on its own and shows the current Spaces and agents.
   - [ ] Restart herdr while the app is open: the message appears, then the app reconnects and re-snapshots. No crash.
   - [ ] No duplicate or spurious notifications after the reconnect.
2. Agent already blocked at start or reconnect
   - [ ] Leave an agent blocked, then launch the app (and again after a herdr restart): it is listed under "Needs you".
   - [ ] No notification fires for it.
   - [ ] An agent that becomes blocked while the app is connected does notify, once.
3. Spaces reordered or closed in herdr's TUI
   - [ ] Reorder Spaces in the TUI: the sidebar order follows.
   - [ ] Close a Space: it disappears from the sidebar together with its agents.
4. Agent pane closes while its panel is open
   - [ ] Panel shows "Session ended".
   - [ ] Reply box, Approve/Deny and terminal input are disabled.
5. Reply content
   - [ ] Multi-line reply arrives intact (check in herdr's TUI).
   - [ ] Unicode (accents, emoji, CJK) arrives intact.
   - [ ] A very long reply (several KB) arrives intact.
   - [ ] Empty and whitespace-only replies leave Send disabled.

## Keyboard only
- [ ] Tab reaches sidebar, agent list, panel, reply box and buttons in a sensible order, with a visible focus ring.
- [ ] Arrow keys move through the agent list.
- [ ] Enter opens the selected agent / sends the reply as designed.
- [ ] Alt+Up / Alt+Down move between agents.
- [ ] In the Terminal tab keys are forwarded to the agent; Cmd+Escape leaves the terminal and focus returns to the panel.

## Accessibility and appearance
- [ ] VoiceOver reads each agent's state label (e.g. working, blocked, done) and announces move/read errors.
- [ ] Dark mode: switch system appearance; all text and states stay legible.

## System integration (bundled build)
- [ ] Dock badge shows the Needs you count and clears at zero.
- [ ] Clicking a notification opens the app with that agent's panel.
- [ ] Closing the window hides it but the app keeps running (agents still update, notifications still fire).
- [ ] Clicking the Dock icon reopens the window.
- [ ] Cmd+Q quits the app.
