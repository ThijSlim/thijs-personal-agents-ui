# thijs-personal-agents-ui

## Agent skills

Agent skills are managed with APM. Run `apm install --legacy-skill-paths` after
cloning or changing `apm.yml`; this deploys to Claude Code's `.claude/skills/`
and Copilot's `.github/skills/`. To refresh upstream versions, run `apm update`
followed by `apm install --legacy-skill-paths`.

## Using it

Agents shows the herdr agents running in your Spaces, lets you answer them, and
notifies you when one needs you. It talks to a running herdr server.

1. Start herdr: `herdr` (or `herdr server`). If it isn't running, the app says
   so and reconnects by itself once it is.
2. Start the app (`pnpm tauri dev`, or open the built `.app`).

- The sidebar lists Spaces and their agents in herdr's order. Agents that are
  blocked and waiting on you also appear under "Needs you", and the Dock badge
  counts them. A notification fires when an agent becomes blocked; clicking it
  opens that agent.
- Select an agent to see its conversation. Type a reply and send it, or use
  Approve / Deny when it asks for permission. Alt+Up / Alt+Down switch agents.
- The Terminal tab shows the agent's screen and forwards your keys. Press
  Cmd+Escape to leave the terminal.
- Closing the window keeps the app running in the background; click the Dock
  icon to reopen it. Cmd+Q quits.
- Logs: `~/Library/Logs/com.thijs.agents/`.
- `HERDR_SOCKET_PATH` overrides which herdr socket the app connects to.

## Develop

Prerequisites: Node 24+, pnpm, Rust (stable) and [herdr](https://herdr.chefgroep.nl/) (`brew install herdr`; developed against herdr 0.9.5).
Homebrew's `rustup` is keg-only, so put it on your PATH:

```sh
export PATH="$HOME/.cargo/bin:/opt/homebrew/opt/rustup/bin:$PATH"
```

```sh
pnpm install
pnpm tauri dev                                  # run the app (window titled "Agents")
pnpm test                                       # frontend tests (vitest)
cargo test --manifest-path src-tauri/Cargo.toml # Rust tests
```

The herdr API schema is captured in `docs/herdr/herdr-api.schema.json`
(`herdr api schema --output ...`).
