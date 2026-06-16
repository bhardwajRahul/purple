# purple hero demo. Drives `purple --demo` (seeded deterministic state in
# src/demo.rs) through every top page. Regenerated in CI on demand so the
# moving demo never drifts from the binary. Run from the repo root:
#   vhs assets/tapes/demo.tape
# CI builds the release binary first; locally the debug binary is fine.

Output demo.webm
Output demo.gif

Set Shell "bash"
Set FontFamily "Berkeley Mono, JetBrains Mono"
Set FontSize 17
Set Width 1400
Set Height 820
Set Padding 22
Set Margin 28
Set MarginFill "#0a0a14"
Set BorderRadius 12
Set CursorBlink false
Set Theme { "name": "purple", "background": "#1b1c26", "foreground": "#e0d6f0", "cursor": "#b44aff", "selection": "#2a2045", "black": "#1b1c26", "red": "#ff2a6d", "green": "#05ffa1", "yellow": "#f0e030", "blue": "#3b8eea", "magenta": "#b44aff", "cyan": "#00f0ff", "white": "#e0d6f0", "brightBlack": "#3d3558", "brightRed": "#ff2a6d", "brightGreen": "#05ffa1", "brightYellow": "#f0e030", "brightBlue": "#3b8eea", "brightMagenta": "#b44aff", "brightCyan": "#00f0ff", "brightWhite": "#ffffff" }

Hide
Type "target/release/purple --demo" Enter
Sleep 2s
Show

# ── Hosts: walk the list with the detail panel open ──
Sleep 1200ms
Type "j" Sleep 900ms
Type "j" Sleep 900ms
Type "j" Sleep 1100ms

# ── Tunnels: live throughput, channel swimlane, per-client roster ──
Tab Sleep 1200ms
Type "j" Sleep 900ms
Type "j" Sleep 1100ms

# ── Containers: fleet-wide, detail panel repaints per row ──
Tab Sleep 1200ms
Type "j" Sleep 800ms
Type "j" Sleep 1100ms

# ── Snippets: walk to the deploy track-record hero ──
Tab Sleep 1200ms
Type "j" Sleep 600ms
Type "j" Sleep 600ms
Type "j" Sleep 600ms
Type "j" Sleep 600ms
Type "j" Sleep 1200ms

# ── Keys: randomart, Vault TTL strip, linked hosts ──
Tab Sleep 1200ms
Type "j" Sleep 900ms
Type "j" Sleep 1300ms

# ── Jump: universal fuzzy bar ──
Tab Sleep 800ms
Type ":" Sleep 1500ms
Escape Sleep 600ms
