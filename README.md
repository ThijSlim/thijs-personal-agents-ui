# thijs-personal-agents-ui

## Agent skills

Agent skills are managed with APM. Run `apm install --legacy-skill-paths` after
cloning or changing `apm.yml`; this deploys to Claude Code's `.claude/skills/`
and Copilot's `.github/skills/`. To refresh upstream versions, run `apm update`
followed by `apm install --legacy-skill-paths`.

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
